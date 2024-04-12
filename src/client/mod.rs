#[cfg(not(any(
    feature = "device_client",
    feature = "module_client",
    feature = "edge_client",
    doc
)))]
compile_error!("Either feature 'device_client' 'module_client' xor 'edge_client' feature must be enabled for this crate.");

#[cfg(all(feature = "device_client", feature = "module_client"))]
compile_error!("Either feature 'device_client' 'module_client' xor 'edge_client' feature must be enabled for this crate.");

#[cfg(all(feature = "device_client", feature = "edge_client"))]
compile_error!("Either feature 'device_client' 'module_client' xor 'edge_client' feature must be enabled for this crate.");

#[cfg(all(feature = "module_client", feature = "edge_client"))]
compile_error!("Either feature 'device_client' 'module_client' xor 'edge_client' feature must be enabled for this crate.");

pub use self::message::{Direction, DispositionResult, IotMessage, IotMessageBuilder};
pub use self::twin::ClientType;
#[cfg(feature = "device_client")]
use self::twin::DeviceTwin;
#[cfg(any(feature = "module_client", feature = "edge_client"))]
use crate::client::twin::ModuleTwin;
use crate::client::twin::Twin;
use anyhow::{ensure, Result};
use async_trait::async_trait;
use azure_iot_sdk_sys::*;
use core::slice;
#[cfg(feature = "module_client")]
use eis_utils::*;
use futures::task;
use log::{debug, error, info, trace, warn};
use serde_json::json;
#[cfg(feature = "module_client")]
use std::time::SystemTime;
use std::{
    boxed::Box,
    env,
    ffi::{c_void, CStr, CString},
    mem, str,
    sync::Once,
    task::{Context, Poll},
};
use tokio::{
    sync::{mpsc, oneshot},
    task::{JoinError, JoinSet},
    time::{timeout, Duration},
};

/// iothub cloud to device (C2D) and device to cloud (D2C) messages
mod message;
/// client implementation, either device, module or edge
mod twin;

/// DirectMethod
pub struct DirectMethod {
    name: String,
    payload: serde_json::Value,
    responder: DirectMethodResponder,
}
/// Result used by iothub client consumer to send the result of a direct method
pub type DirectMethodResponder = oneshot::Sender<Result<Option<serde_json::Value>>>;
/// Sender used to signal a direct method to the iothub client consumer
pub type DirectMethodSender = mpsc::Sender<DirectMethod>;

/// Trait which provides functions for communication with azure iothub
#[async_trait(?Send)]
pub trait IotHub {
    /// Call this function to get a builder for an IotHub.
    fn builder() -> Box<dyn IotHubBuilder>
    where
        Self: Sized;

    /// Call this function in order to get the underlying azure-sdk-c version string.
    fn sdk_version_string() -> String
    where
        Self: Sized;

    /// Call this function to get the configured [`ClientType`].
    fn client_type() -> ClientType
    where
        Self: Sized;

    /// Call this function to trigger a twin update that is asynchronously signaled as twin_desired stream.
    fn twin_async(&mut self) -> Result<()>;

    /// Call this function to send a message (D2C) to iothub.
    fn send_d2c_message(&mut self, message: IotMessage) -> Result<()>;

    /// Call this function to report twin properties to iothub.
    fn twin_report(&mut self, reported: serde_json::Value) -> Result<()>;

    /// Call this function to properly shutdown IotHub. All reported properties and D2C messages will be
    /// continued to completion.
    async fn shutdown(&mut self);
}

/// Trait which provides functions for communication with azure iothub
#[async_trait(?Send)]
pub trait IotHubBuilder {
    #[cfg(feature = "edge_client")]
    /// Call this function in order to build an instance of an edge client based [`IotHubClient`].
    fn build_edge_client(&self) -> Result<Box<dyn IotHub>>;

    #[cfg(feature = "device_client")]
    /// Call this function in order to build an instance of a device client based [`IotHubClient`].
    fn build_device_client(&self, connection_string: &str) -> Result<Box<dyn IotHub>>;

    #[cfg(feature = "module_client")]
    /// Call this function in order to build an instance of a module client based [`IotHubClient`] by connection string.
    fn build_module_client(&self, connection_string: &str) -> Result<Box<dyn IotHub>>;

    #[cfg(feature = "module_client")]
    /// Call this function in order to build an instance of a module client based [`IotHubClient`] by identity service.
    async fn build_module_client_from_identity(&self) -> Result<Box<dyn IotHub>>;

    /// Call this function to observe connection state.
    fn observe_connection_state(
        self: Box<Self>,
        tx_connection_status: mpsc::Sender<AuthenticationStatus>,
    ) -> Box<dyn IotHubBuilder>;

    /// Call this function to observe desired properties.
    fn observe_desired_properties(
        self: Box<Self>,
        tx_twin_desired: mpsc::Sender<(TwinUpdateState, serde_json::Value)>,
    ) -> Box<dyn IotHubBuilder>;

    /// Call this function to observe direct methods.
    fn observe_direct_methods(
        self: Box<Self>,
        tx_direct_method: DirectMethodSender,
    ) -> Box<dyn IotHubBuilder>;

    /// Call this function to observe incoming messages.
    fn observe_incoming_messages(
        self: Box<Self>,
        tx_incoming_message: IncomingMessageObserver,
    ) -> Box<dyn IotHubBuilder>;
}

static AZURE_SDK_LOGGING: &str = "AZURE_SDK_LOGGING";
static AZURE_SDK_DO_WORK_FREQUENCY_IN_MS: &str = "AZURE_SDK_DO_WORK_FREQUENCY_IN_MS";
static DO_WORK_FREQUENCY_RANGE_IN_MS: std::ops::RangeInclusive<u64> = 0..=100;
static DO_WORK_FREQUENCY_DEFAULT_IN_MS: u64 = 100;
static AZURE_SDK_CONFIRMATION_TIMEOUT_IN_SECS: &str = "AZURE_SDK_CONFIRMATION_TIMEOUT_IN_SECS";
static CONFIRMATION_TIMEOUT_DEFAULT_IN_SECS: u64 = 300;

#[cfg(feature = "module_client")]
macro_rules! days_to_secs {
    ($num_days:expr) => {
        $num_days * 24 * 60 * 60
    };
}

/// Indicates [type](https://docs.microsoft.com/en-us/azure/iot-hub/iot-hub-devguide-module-twins#back-end-operations) of desired properties update
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TwinUpdateState {
    /// complete update of desired properties
    Complete = 0,
    /// partial update of desired properties
    Partial = 1,
}

/// Reason for unauthenticated connection result
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum UnauthenticatedReason {
    /// SAS token expired
    ExpiredSasToken,
    /// device is disabled in iothub
    DeviceDisabled,
    /// invalid credentials detected by iothub
    BadCredential,
    /// connection retry expired
    RetryExpired,
    /// no network
    NoNetwork,
    /// other communication error
    CommunicationError,
}

/// Authentication status as a result of establishing a connection
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AuthenticationStatus {
    /// authenticated successfully
    Authenticated,
    /// authenticated not successfully with unauthenticated reason
    Unauthenticated(UnauthenticatedReason),
}

/// Provides a channel and a property array to receive incoming cloud to device messages
#[derive(Clone, Debug)]
pub struct IncomingMessageObserver {
    observer: mpsc::Sender<(IotMessage, oneshot::Sender<Result<DispositionResult>>)>,
    properties: Vec<String>,
}

impl IncomingMessageObserver {
    /// Creates a new instance of [`IncomingMessageObserver`]
    pub fn new(
        observer: mpsc::Sender<(IotMessage, oneshot::Sender<Result<DispositionResult>>)>,
        properties: Vec<String>,
    ) -> Self {
        IncomingMessageObserver {
            observer,
            properties,
        }
    }
}

/// Builder used to create an instance of [`IotHubClient`]
/// ```no_run
/// use azure_iot_sdk::client::*;
/// use std::{thread, time};
/// use tokio::{select, sync::mpsc};
///
/// #[tokio::main]
/// async fn main() {
///     let (tx_connection_status, mut rx_connection_status) = mpsc::channel(100);
///     let (tx_twin_desired, mut rx_twin_desired) = mpsc::channel(100);
///     let (tx_direct_method, mut rx_direct_method) = mpsc::channel(100);
///     let (tx_incoming_message, mut rx_incoming_message) = mpsc::channel(100);
///     let builder = IotHubClient::builder()
///         .observe_connection_state(tx_connection_status)
///         .observe_desired_properties(tx_twin_desired)
///         .observe_direct_methods(tx_direct_method)
///         .observe_incoming_messages(IncomingMessageObserver::new(tx_incoming_message, vec![]));
///
///     #[cfg(feature = "edge_client")]
///     let mut client = builder.build_edge_client().unwrap();
///     #[cfg(feature = "device_client")]
///     let mut client = builder.build_device_client("my-connection-string").unwrap();
///     #[cfg(feature = "module_client")]
///     let mut client = builder.build_module_client("my-connection-string").unwrap();
///
///     loop {
///         select! (
///             status = rx_connection_status.recv() => {
///                 // handle connection status;
///                 // ...
///             },
///             status = rx_twin_desired.recv() => {
///                 // handle twin desired properties;
///                 // ...
///             },
///             status = rx_direct_method.recv() => {
///                 // handle direct method calls;
///                 // ...
///             },
///             status = rx_incoming_message.recv() => {
///                 // handle cloud to device messages;
///                 // ...
///             },
///         )
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct IotHubClientBuilder {
    tx_connection_status: Option<mpsc::Sender<AuthenticationStatus>>,
    tx_twin_desired: Option<mpsc::Sender<(TwinUpdateState, serde_json::Value)>>,
    tx_direct_method: Option<DirectMethodSender>,
    tx_incoming_message: Option<IncomingMessageObserver>,
}

#[async_trait(?Send)]
impl IotHubBuilder for IotHubClientBuilder {
    #[cfg(feature = "edge_client")]
    /// Call this function in order to build an instance of an edge client based [`IotHubClient`].<br>
    /// ***Note***: this function is only available with "edge_client" feature enabled.
    /// ```no_run
    /// use azure_iot_sdk::client::*;
    /// use std::{thread, time};
    /// use tokio::{select, sync::mpsc};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let (tx_connection_status, mut rx_connection_status) = mpsc::channel(100);
    ///     let (tx_twin_desired, mut rx_twin_desired) = mpsc::channel(100);
    ///     let (tx_direct_method, mut rx_direct_method) = mpsc::channel(100);
    ///     let (tx_incoming_message, mut rx_incoming_message) = mpsc::channel(100);
    ///
    ///     let mut client = IotHubClient::builder()
    ///         .observe_connection_state(tx_connection_status)
    ///         .observe_desired_properties(tx_twin_desired)
    ///         .observe_direct_methods(tx_direct_method)
    ///         .observe_incoming_messages(IncomingMessageObserver::new(tx_incoming_message, vec![]))
    ///         .build_edge_client()
    ///         .unwrap();
    ///
    ///     loop {
    ///         select! (
    ///             status = rx_connection_status.recv() => {
    ///                 // handle connection status;
    ///                 // ...
    ///             },
    ///             status = rx_twin_desired.recv() => {
    ///                 // handle twin desired properties;
    ///                 // ...
    ///             },
    ///             status = rx_direct_method.recv() => {
    ///                 // handle direct method calls;
    ///                 // ...
    ///             },
    ///             status = rx_incoming_message.recv() => {
    ///                 // handle cloud to device messages;
    ///                 // ...
    ///             },
    ///         )
    ///     }
    /// }
    /// ```
    fn build_edge_client(&self) -> Result<Box<dyn IotHub>> {
        ensure!(
            ClientType::Edge == IotHubClient::client_type(),
            "edge client type requires edge_client feature"
        );

        IotHubClient::from_edge_environment(
            self.tx_connection_status.clone(),
            self.tx_twin_desired.clone(),
            self.tx_direct_method.clone(),
            self.tx_incoming_message.clone(),
        )
    }

    #[cfg(feature = "device_client")]
    /// Call this function in order to build an instance of a device client based [`IotHubClient`].<br>
    /// ***Note***: this function is only available with "device_client" feature enabled.
    /// ```no_run
    /// use azure_iot_sdk::client::*;
    /// use std::{thread, time};
    /// use tokio::{select, sync::mpsc};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let (tx_connection_status, mut rx_connection_status) = mpsc::channel(100);
    ///     let (tx_twin_desired, mut rx_twin_desired) = mpsc::channel(100);
    ///     let (tx_direct_method, mut rx_direct_method) = mpsc::channel(100);
    ///     let (tx_incoming_message, mut rx_incoming_message) = mpsc::channel(100);
    ///
    ///     let mut client = IotHubClient::builder()
    ///         .observe_connection_state(tx_connection_status)
    ///         .observe_desired_properties(tx_twin_desired)
    ///         .observe_direct_methods(tx_direct_method)
    ///         .observe_incoming_messages(IncomingMessageObserver::new(tx_incoming_message, vec![]))
    ///         .build_device_client("my_connection_string")
    ///         .unwrap();
    ///
    ///     loop {
    ///         select! (
    ///             status = rx_connection_status.recv() => {
    ///                 // handle connection status;
    ///                 // ...
    ///             },
    ///             status = rx_twin_desired.recv() => {
    ///                 // handle twin desired properties;
    ///                 // ...
    ///             },
    ///             status = rx_direct_method.recv() => {
    ///                 // handle direct method calls;
    ///                 // ...
    ///             },
    ///             status = rx_incoming_message.recv() => {
    ///                 // handle cloud to device messages;
    ///                 // ...
    ///             },
    ///         )
    ///     }
    /// }
    /// ```
    fn build_device_client(&self, connection_string: &str) -> Result<Box<dyn IotHub>> {
        ensure!(
            ClientType::Device == IotHubClient::client_type(),
            "device client type requires device_client feature"
        );

        IotHubClient::from_connection_string(
            connection_string,
            self.tx_connection_status.clone(),
            self.tx_twin_desired.clone(),
            self.tx_direct_method.clone(),
            self.tx_incoming_message.clone(),
        )
    }

    #[cfg(feature = "module_client")]
    /// Call this function in order to build an instance of a module client based [`IotHubClient`] by connection string.<br>
    /// ***Note***: this function is only available with "module_client" feature enabled.
    /// ```no_run
    /// use azure_iot_sdk::client::*;
    /// use std::{thread, time};
    /// use tokio::{select, sync::mpsc};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let (tx_connection_status, mut rx_connection_status) = mpsc::channel(100);
    ///     let (tx_twin_desired, mut rx_twin_desired) = mpsc::channel(100);
    ///     let (tx_direct_method, mut rx_direct_method) = mpsc::channel(100);
    ///     let (tx_incoming_message, mut rx_incoming_message) = mpsc::channel(100);
    ///
    ///     let mut client = IotHubClient::builder()
    ///         .observe_connection_state(tx_connection_status)
    ///         .observe_desired_properties(tx_twin_desired)
    ///         .observe_direct_methods(tx_direct_method)
    ///         .observe_incoming_messages(IncomingMessageObserver::new(tx_incoming_message, vec![]))
    ///         .build_module_client("my_connection_string")
    ///         .unwrap();
    ///
    ///     loop {
    ///         select! (
    ///             status = rx_connection_status.recv() => {
    ///                 // handle connection status;
    ///                 // ...
    ///             },
    ///             status = rx_twin_desired.recv() => {
    ///                 // handle twin desired properties;
    ///                 // ...
    ///             },
    ///             status = rx_direct_method.recv() => {
    ///                 // handle direct method calls;
    ///                 // ...
    ///             },
    ///             status = rx_incoming_message.recv() => {
    ///                 // handle cloud to device messages;
    ///                 // ...
    ///             },
    ///         )
    ///     }
    /// }
    /// ```
    fn build_module_client(&self, connection_string: &str) -> Result<Box<dyn IotHub>> {
        ensure!(
            ClientType::Module == IotHubClient::client_type(),
            "module client type requires module_client feature"
        );

        IotHubClient::from_connection_string(
            connection_string,
            self.tx_connection_status.clone(),
            self.tx_twin_desired.clone(),
            self.tx_direct_method.clone(),
            self.tx_incoming_message.clone(),
        )
    }

    #[cfg(feature = "module_client")]
    /// Call this function in order to build an instance of a module client based [`IotHubClient`].<br>
    /// ***Note1***: this function gets its connection string from identity service.<br>
    /// ***Note2***: this function is only available with "module_client" feature enabled.
    /// ```no_run
    /// use azure_iot_sdk::client::*;
    /// use std::{thread, time};
    /// use tokio::{select, sync::mpsc};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let (tx_connection_status, mut rx_connection_status) = mpsc::channel(100);
    ///     let (tx_twin_desired, mut rx_twin_desired) = mpsc::channel(100);
    ///     let (tx_direct_method, mut rx_direct_method) = mpsc::channel(100);
    ///     let (tx_incoming_message, mut rx_incoming_message) = mpsc::channel(100);
    ///
    ///     let mut client = IotHubClient::builder()
    ///         .observe_connection_state(tx_connection_status)
    ///         .observe_desired_properties(tx_twin_desired)
    ///         .observe_direct_methods(tx_direct_method)
    ///         .observe_incoming_messages(IncomingMessageObserver::new(tx_incoming_message, vec![]))
    ///         .build_module_client_from_identity()
    ///         .await
    ///         .unwrap();
    ///
    ///     loop {
    ///         select! (
    ///             status = rx_connection_status.recv() => {
    ///                 // handle connection status;
    ///                 // ...
    ///             },
    ///             status = rx_twin_desired.recv() => {
    ///                 // handle twin desired properties;
    ///                 // ...
    ///             },
    ///             status = rx_direct_method.recv() => {
    ///                 // handle direct method calls;
    ///                 // ...
    ///             },
    ///             status = rx_incoming_message.recv() => {
    ///                 // handle cloud to device messages;
    ///                 // ...
    ///             },
    ///         )
    ///     }
    /// }
    /// ```
    async fn build_module_client_from_identity(&self) -> Result<Box<dyn IotHub>> {
        ensure!(
            ClientType::Module == IotHubClient::client_type(),
            "module client type requires module_client feature"
        );

        IotHubClient::from_identity_service(
            self.tx_connection_status.clone(),
            self.tx_twin_desired.clone(),
            self.tx_direct_method.clone(),
            self.tx_incoming_message.clone(),
        )
        .await
    }

    /// Add connection state observer
    /// ```no_run
    /// use azure_iot_sdk::client::*;
    /// use std::{thread, time};
    /// use tokio::{select, sync::mpsc};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let (tx_connection_status, mut rx_connection_status) = mpsc::channel(100);
    ///
    ///     #[cfg(feature = "edge_client")]
    ///     let mut client = IotHubClient::builder().observe_connection_state(tx_connection_status).build_edge_client().unwrap();
    ///     #[cfg(feature = "device_client")]
    ///     let mut client = IotHubClient::builder().observe_connection_state(tx_connection_status).build_device_client("my-connection-string").unwrap();
    ///     #[cfg(feature = "module_client")]
    ///     let mut client = IotHubClient::builder().observe_connection_state(tx_connection_status).build_module_client("my-connection-string").unwrap();
    ///
    ///     loop {
    ///         select! (
    ///             status = rx_connection_status.recv() => {
    ///                 // handle connection status;
    ///                 // ...
    ///             },
    ///         )
    ///     }
    /// }
    /// ```
    fn observe_connection_state(
        mut self: Box<Self>,
        tx_connection_status: mpsc::Sender<AuthenticationStatus>,
    ) -> Box<dyn IotHubBuilder> {
        self.tx_connection_status = Some(tx_connection_status);
        self
    }

    /// Add desired properties observer
    /// ```no_run
    /// use azure_iot_sdk::client::*;
    /// use std::{thread, time};
    /// use tokio::{select, sync::mpsc};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let (tx_twin_desired, mut rx_twin_desired) = mpsc::channel(100);
    ///
    ///     #[cfg(feature = "edge_client")]
    ///     let mut client = IotHubClient::builder().observe_desired_properties(tx_twin_desired).build_edge_client().unwrap();
    ///     #[cfg(feature = "device_client")]
    ///     let mut client = IotHubClient::builder().observe_desired_properties(tx_twin_desired).build_device_client("my-connection-string").unwrap();
    ///     #[cfg(feature = "module_client")]
    ///     let mut client = IotHubClient::builder().observe_desired_properties(tx_twin_desired).build_module_client("my-connection-string").unwrap();
    ///
    ///     loop {
    ///         select! (
    ///             status = rx_twin_desired.recv() => {
    ///                 // handle twin desired properties;
    ///                 // ...
    ///             },
    ///         )
    ///     }
    /// }
    /// ```
    fn observe_desired_properties(
        mut self: Box<Self>,
        tx_twin_desired: mpsc::Sender<(TwinUpdateState, serde_json::Value)>,
    ) -> Box<dyn IotHubBuilder> {
        self.tx_twin_desired = Some(tx_twin_desired);
        self
    }

    /// Add direct method observer
    /// ```no_run
    /// use azure_iot_sdk::client::*;
    /// use std::{thread, time};
    /// use tokio::{select, sync::mpsc};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let (tx_direct_method, mut rx_direct_method) = mpsc::channel(100);
    ///
    ///     #[cfg(feature = "edge_client")]
    ///     let mut client = IotHubClient::builder()
    ///         .observe_direct_methods(tx_direct_method)
    ///         .build_edge_client()
    ///         .unwrap();
    ///     #[cfg(feature = "device_client")]
    ///     let mut client = IotHubClient::builder()
    ///         .observe_direct_methods(tx_direct_method)
    ///         .build_device_client("my-connection-string")
    ///         .unwrap();
    ///     #[cfg(feature = "module_client")]
    ///     let mut client = IotHubClient::builder()
    ///         .observe_direct_methods(tx_direct_method)
    ///         .build_module_client("my-connection-string")
    ///         .unwrap();
    ///
    ///     loop {
    ///         select! (
    ///             status = rx_direct_method.recv() => {
    ///                 // handle direct method calls;
    ///                 // ...
    ///             },
    ///         )
    ///     }
    /// }
    /// ```
    fn observe_direct_methods(
        mut self: Box<Self>,
        tx_direct_method: DirectMethodSender,
    ) -> Box<dyn IotHubBuilder> {
        self.tx_direct_method = Some(tx_direct_method);
        self
    }

    /// Add incoming message observer
    /// ```no_run
    /// use azure_iot_sdk::client::*;
    /// use std::{thread, time};
    /// use tokio::{select, sync::mpsc};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let (tx_incoming_message, mut rx_incoming_message) = mpsc::channel(100);
    ///
    ///     #[cfg(feature = "edge_client")]
    ///     let mut client = IotHubClient::builder()
    ///         .observe_incoming_messages(IncomingMessageObserver::new(tx_incoming_message, vec![]))
    ///         .build_edge_client()
    ///         .unwrap();
    ///     #[cfg(feature = "device_client")]
    ///     let mut client = IotHubClient::builder()
    ///         .observe_incoming_messages(IncomingMessageObserver::new(tx_incoming_message, vec![]))
    ///         .build_device_client("my-connection-string")
    ///         .unwrap();
    ///     #[cfg(feature = "module_client")]
    ///     let mut client = IotHubClient::builder()
    ///         .observe_incoming_messages(IncomingMessageObserver::new(tx_incoming_message, vec![]))
    ///         .build_module_client("my-connection-string")
    ///         .unwrap();
    ///
    ///     loop {
    ///         select! (
    ///             status = rx_incoming_message.recv() => {
    ///                 // handle cloud to device messages;
    ///                 // ...
    ///             },
    ///         )
    ///     }
    /// }
    /// ```
    fn observe_incoming_messages(
        mut self: Box<Self>,
        tx_incoming_message: IncomingMessageObserver,
    ) -> Box<dyn IotHubBuilder> {
        self.tx_incoming_message = Some(tx_incoming_message);
        self
    }
}

/// iothub client to be instantiated in order to initiate iothub communication
/// ```no_run
/// use azure_iot_sdk::client::*;
/// use std::{thread, time};
/// use tokio::{select, sync::mpsc};
///
/// #[tokio::main]
/// async fn main() {
///     let (tx_connection_status, mut rx_connection_status) = mpsc::channel(100);
///     let (tx_twin_desired, mut rx_twin_desired) = mpsc::channel(100);
///     let (tx_direct_method, mut rx_direct_method) = mpsc::channel(100);
///     let (tx_incoming_message, mut rx_incoming_message) = mpsc::channel(100);
///     let builder = IotHubClient::builder()
///         .observe_connection_state(tx_connection_status)
///         .observe_desired_properties(tx_twin_desired)
///         .observe_direct_methods(tx_direct_method)
///         .observe_incoming_messages(IncomingMessageObserver::new(tx_incoming_message, vec![]));
///
///     #[cfg(feature = "edge_client")]
///     let mut client = builder.build_edge_client().unwrap();
///     #[cfg(feature = "device_client")]
///     let mut client = builder.build_device_client("my-connection-string").unwrap();
///     #[cfg(feature = "module_client")]
///     let mut client = builder.build_module_client("my-connection-string").unwrap();
///
///     loop {
///         select! (
///             status = rx_connection_status.recv() => {
///                 // handle connection status;
///                 // ...
///             },
///             status = rx_twin_desired.recv() => {
///                 // handle twin desired properties;
///                 // ...
///             },
///             status = rx_direct_method.recv() => {
///                 // handle direct method calls;
///                 // ...
///             },
///             status = rx_incoming_message.recv() => {
///                 // handle cloud to device messages;
///                 // ...
///             },
///         )
///     }
/// }
/// ```
pub struct IotHubClient {
    twin: Box<dyn Twin>,
    tx_connection_status: Option<mpsc::Sender<AuthenticationStatus>>,
    tx_twin_desired: Option<mpsc::Sender<(TwinUpdateState, serde_json::Value)>>,
    tx_direct_method: Option<DirectMethodSender>,
    tx_incoming_message: Option<IncomingMessageObserver>,
    confirmation_set: JoinSet<()>,
}

#[async_trait(?Send)]
impl IotHub for IotHubClient {
    /// Call this function in order to get the underlying azure-sdk-c version string.
    /// ```rust, no_run
    /// use azure_iot_sdk::client::*;
    ///
    /// IotHubClient::sdk_version_string();
    /// ```
    fn sdk_version_string() -> String {
        twin::sdk_version_string()
    }

    /// Call this function to get the configured [`ClientType`].
    /// ```rust, no_run
    /// use azure_iot_sdk::client::*;
    ///
    /// IotHubClient::client_type();
    /// ```
    fn client_type() -> ClientType {
        if cfg!(feature = "device_client") {
            ClientType::Device
        } else if cfg!(feature = "module_client") {
            ClientType::Module
        } else if cfg!(feature = "edge_client") {
            ClientType::Edge
        } else {
            panic!("no client type feature set")
        }
    }

    fn builder() -> Box<dyn IotHubBuilder> {
        Box::new(IotHubClientBuilder::default())
    }

    /// Call this function to send a message (D2C) to iothub.
    /// ```rust, no_run
    /// use azure_iot_sdk::client::*;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     #[cfg(feature = "edge_client")]
    ///     let mut client = IotHubClient::builder().build_edge_client().unwrap();
    ///     #[cfg(feature = "device_client")]
    ///     let mut client = IotHubClient::builder().build_device_client("my-connection-string").unwrap();
    ///     #[cfg(feature = "module_client")]
    ///     let mut client = IotHubClient::builder().build_module_client("my-connection-string").unwrap();
    ///
    ///     let msg = IotMessage::builder()
    ///         .set_body(
    ///             serde_json::to_vec(r#"{"my telemetry message": "hi from device"}"#).unwrap(),
    ///         )
    ///         .set_id("my msg id")
    ///         .set_correlation_id("my correleation id")
    ///         .set_property(
    ///             "my property key",
    ///             "my property value",
    ///         )
    ///         .set_output_queue("my output queue")
    ///         .build()
    ///         .unwrap();
    ///
    ///     client.send_d2c_message(msg);
    /// }
    /// ```
    fn send_d2c_message(&mut self, mut message: IotMessage) -> Result<()> {
        let handle = message.create_outgoing_handle()?;
        let queue = message.output_queue.clone();
        let (tx, rx) = oneshot::channel::<bool>();

        debug!("send_d2c_message: {queue:?}");

        self.twin.send_event_to_output_async(
            handle,
            queue,
            Some(IotHubClient::c_d2c_confirmation_callback),
            Box::into_raw(Box::new(tx)) as *mut c_void,
        )?;

        self.spawn_confirmation(rx);

        Ok(())
    }

    /// Call this function to report twin properties to iothub.
    /// ```rust, no_run
    /// use azure_iot_sdk::client::*;
    /// use serde_json::json;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     #[cfg(feature = "edge_client")]
    ///     let mut client = IotHubClient::builder().build_edge_client().unwrap();
    ///     #[cfg(feature = "device_client")]
    ///     let mut client = IotHubClient::builder().build_device_client("my-connection-string").unwrap();
    ///     #[cfg(feature = "module_client")]
    ///     let mut client = IotHubClient::builder().build_module_client("my-connection-string").unwrap();
    ///
    ///     let reported = json!({
    ///         "my_status": {
    ///             "status": "ok",
    ///             "timestamp": "2022-03-10",
    ///         }
    ///     });
    ///
    ///     client.twin_report(reported);
    /// }
    /// ```
    fn twin_report(&mut self, reported: serde_json::Value) -> Result<()> {
        debug!("send reported: {reported:?}");

        let reported_state = CString::new(reported.to_string())?;
        let size = reported_state.as_bytes().len();
        let (tx, rx) = oneshot::channel::<bool>();

        self.twin.send_reported_state(
            reported_state,
            size,
            Some(IotHubClient::c_reported_twin_callback),
            Box::into_raw(Box::new(tx)) as *mut c_void,
        )?;

        self.spawn_confirmation(rx);

        Ok(())
    }

    /// Call this function to trigger a twin update that is asynchronously signaled as twin_desired stream.
    /// ```rust, no_run
    /// use azure_iot_sdk::client::*;
    /// use serde_json::json;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     #[cfg(feature = "edge_client")]
    ///     let mut client = IotHubClient::builder().build_edge_client().unwrap();
    ///     #[cfg(feature = "device_client")]
    ///     let mut client = IotHubClient::builder().build_device_client("my-connection-string").unwrap();
    ///     #[cfg(feature = "module_client")]
    ///     let mut client = IotHubClient::builder().build_module_client("my-connection-string").unwrap();
    ///
    ///     client.twin_async();
    /// }
    /// ```
    fn twin_async(&mut self) -> Result<()> {
        debug!("twin_complete: get entire twin");

        let context = self as *mut IotHubClient as *mut c_void;

        self.twin
            .twin_async(Some(IotHubClient::c_twin_callback), context)
    }

    /// Call this function to properly shutdown IotHub. All reported properties and D2C messages will be
    /// continued to completion.
    /// ```rust, no_run
    /// use azure_iot_sdk::client::*;
    /// use serde_json::json;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     #[cfg(feature = "edge_client")]
    ///     let mut client = IotHubClient::builder().build_edge_client().unwrap();
    ///     #[cfg(feature = "device_client")]
    ///     let mut client = IotHubClient::builder().build_device_client("my-connection-string").unwrap();
    ///     #[cfg(feature = "module_client")]
    ///     let mut client = IotHubClient::builder().build_module_client("my-connection-string").unwrap();
    ///
    ///     let reported = json!({
    ///         "my_status": {
    ///             "status": "ok",
    ///             "timestamp": "2022-03-10",
    ///         }
    ///     });
    ///
    ///     client.twin_report(reported);
    ///
    ///     client.shutdown();
    /// }
    /// ```
    async fn shutdown(&mut self) {
        info!("shutdown");

        /*
           We abort and join all "wait for pending confirmations" tasks
           (https://docs.rs/tokio/latest/src/tokio/task/join_set.rs.html#362).
           This means:
               - we have a clean shutdown
               - we shutdown as fast as possible
               - we DON'T WAIT for pending confirmations
        */

        self.confirmation_set.shutdown().await;
    }
}

impl IotHubClient {
    #[cfg(feature = "edge_client")]
    pub(crate) fn from_edge_environment(
        tx_connection_status: Option<mpsc::Sender<AuthenticationStatus>>,
        tx_twin_desired: Option<mpsc::Sender<(TwinUpdateState, serde_json::Value)>>,
        tx_direct_method: Option<DirectMethodSender>,
        tx_incoming_message: Option<IncomingMessageObserver>,
    ) -> Result<Box<dyn IotHub>> {
        #[cfg(not(feature = "edge_client"))]
        anyhow::bail!(
            "only edge modules can connect via from_edge_environment(). either use from_identity_service() or from_connection_string().",
        );

        #[cfg(feature = "edge_client")]
        {
            IotHubClient::iothub_init()?;

            let mut twin = Box::<ModuleTwin>::default();

            twin.create_from_edge_environment()?;

            let mut client = Box::new(IotHubClient {
                twin,
                tx_connection_status,
                tx_twin_desired,
                tx_direct_method,
                tx_incoming_message,
                confirmation_set: JoinSet::new(),
            });

            client.set_callbacks()?;

            client.set_options()?;

            Ok(client)
        }
    }

    #[cfg(feature = "module_client")]
    pub(crate) async fn from_identity_service(
        _tx_connection_status: Option<mpsc::Sender<AuthenticationStatus>>,
        _tx_twin_desired: Option<mpsc::Sender<(TwinUpdateState, serde_json::Value)>>,
        _tx_direct_method: Option<DirectMethodSender>,
        _tx_incoming_message: Option<IncomingMessageObserver>,
    ) -> Result<Box<dyn IotHub>> {
        let connection_info = request_connection_string_from_eis_with_expiry(
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)?
                    .saturating_add(Duration::from_secs(days_to_secs!(30))),
            ).await
            .map_err(|err| {
                if cfg!(device_client) {
                    error!("iot identity service failed to create device client identity.
                    In case you use TPM attestation please note that this combination is currently not supported.");
                }

                err
            })?;

        debug!(
            "used con_str: {}",
            connection_info.connection_string.as_str()
        );

        IotHubClient::from_connection_string(
            connection_info.connection_string.as_str(),
            _tx_connection_status,
            _tx_twin_desired,
            _tx_direct_method,
            _tx_incoming_message,
        )
    }

    #[cfg(any(feature = "module_client", feature = "device_client"))]
    pub(crate) fn from_connection_string(
        connection_string: &str,
        tx_connection_status: Option<mpsc::Sender<AuthenticationStatus>>,
        tx_twin_desired: Option<mpsc::Sender<(TwinUpdateState, serde_json::Value)>>,
        tx_direct_method: Option<DirectMethodSender>,
        tx_incoming_message: Option<IncomingMessageObserver>,
    ) -> Result<Box<dyn IotHub + 'static>> {
        IotHubClient::iothub_init()?;

        #[cfg(feature = "module_client")]
        let mut twin = Box::<ModuleTwin>::default();

        #[cfg(feature = "device_client")]
        let mut twin = Box::<DeviceTwin>::default();

        twin.create_from_connection_string(CString::new(connection_string)?)?;

        let mut client = Box::new(IotHubClient {
            twin,
            tx_connection_status,
            tx_twin_desired,
            tx_direct_method,
            tx_incoming_message,
            confirmation_set: JoinSet::new(),
        });

        client.set_callbacks()?;

        client.set_options()?;

        Ok(client)
    }

    fn iothub_init() -> Result<()> {
        static IOTHUB_INIT_ONCE: Once = Once::new();
        static mut IOTHUB_INIT_RESULT: i32 = -1;

        unsafe {
            IOTHUB_INIT_ONCE.call_once(|| {
                IOTHUB_INIT_RESULT = IoTHub_Init();
            });

            match IOTHUB_INIT_RESULT {
                0 => Ok(()),
                _ => anyhow::bail!("error while IoTHub_Init()"),
            }
        }
    }

    fn set_callbacks(&mut self) -> Result<()> {
        let context = self as *mut IotHubClient as *mut c_void;

        if self.tx_connection_status.is_some() {
            self.twin.set_connection_status_callback(
                Some(IotHubClient::c_connection_status_callback),
                context,
            )?;
        }

        if self.tx_incoming_message.is_some() {
            self.twin
                .set_input_message_callback(Some(IotHubClient::c_c2d_message_callback), context)?;
        }

        if self.tx_twin_desired.is_some() {
            self.twin
                .set_twin_callback(Some(IotHubClient::c_twin_callback), context)?;
        }
        if self.tx_direct_method.is_some() {
            self.twin
                .set_method_callback(Some(IotHubClient::c_direct_method_callback), context)?;
        }

        Ok(())
    }

    fn set_options(&mut self) -> Result<()> {
        let mut do_work_freq = None;

        if let Ok(freq) = env::var(AZURE_SDK_DO_WORK_FREQUENCY_IN_MS) {
            match freq.parse::<u64>() {
                Ok(freq) if DO_WORK_FREQUENCY_RANGE_IN_MS.contains(&freq) => {
                    info!("set do_work frequency {freq}ms");
                    do_work_freq = Some(freq);
                }
                _ => error!("ignore do_work frequency {freq} since not in range of {DO_WORK_FREQUENCY_RANGE_IN_MS:?}ms"),
            };
        }

        if do_work_freq.is_none() {
            do_work_freq = Some(DO_WORK_FREQUENCY_DEFAULT_IN_MS);
            info!("set default do_work frequency {DO_WORK_FREQUENCY_DEFAULT_IN_MS}ms")
        }

        self.twin.set_option(
            CString::new("do_work_freq_ms")?,
            do_work_freq.as_mut().unwrap() as *const uint_fast64_t as *const c_void,
        )?;

        if env::var(AZURE_SDK_LOGGING).is_ok() {
            self.twin.set_option(
                CString::new("logtrace")?,
                &mut true as *const bool as *const c_void,
            )?
        }

        let model_id = CString::new("dtmi:azure:iot:deviceUpdateContractModel;3")?;
        self.twin.set_option(
            CString::new("model_id")?,
            model_id.as_ptr() as *const c_void,
        )?;

        Ok(())
    }

    unsafe extern "C" fn c_connection_status_callback(
        connection_status: IOTHUB_CLIENT_CONNECTION_STATUS,
        status_reason: IOTHUB_CLIENT_CONNECTION_STATUS_REASON,
        context: *mut ::std::os::raw::c_void,
    ) {
        let client = &mut *(context as *mut IotHubClient);

        let status = match connection_status {
            IOTHUB_CLIENT_CONNECTION_STATUS_TAG_IOTHUB_CLIENT_CONNECTION_AUTHENTICATED => {
                AuthenticationStatus::Authenticated
            }
            IOTHUB_CLIENT_CONNECTION_STATUS_TAG_IOTHUB_CLIENT_CONNECTION_UNAUTHENTICATED => {
                match status_reason {
                    IOTHUB_CLIENT_CONNECTION_STATUS_REASON_TAG_IOTHUB_CLIENT_CONNECTION_EXPIRED_SAS_TOKEN => {
                        AuthenticationStatus::Unauthenticated(
                            UnauthenticatedReason::ExpiredSasToken,
                        )
                    }
                    IOTHUB_CLIENT_CONNECTION_STATUS_REASON_TAG_IOTHUB_CLIENT_CONNECTION_DEVICE_DISABLED => {
                        AuthenticationStatus::Unauthenticated(UnauthenticatedReason::DeviceDisabled)
                    }
                    IOTHUB_CLIENT_CONNECTION_STATUS_REASON_TAG_IOTHUB_CLIENT_CONNECTION_BAD_CREDENTIAL => {
                        AuthenticationStatus::Unauthenticated(UnauthenticatedReason::BadCredential)
                    }
                    IOTHUB_CLIENT_CONNECTION_STATUS_REASON_TAG_IOTHUB_CLIENT_CONNECTION_RETRY_EXPIRED => {
                        AuthenticationStatus::Unauthenticated(UnauthenticatedReason::RetryExpired)
                    }
                    IOTHUB_CLIENT_CONNECTION_STATUS_REASON_TAG_IOTHUB_CLIENT_CONNECTION_NO_NETWORK => {
                        AuthenticationStatus::Unauthenticated(UnauthenticatedReason::NoNetwork)
                    }
                    IOTHUB_CLIENT_CONNECTION_STATUS_REASON_TAG_IOTHUB_CLIENT_CONNECTION_COMMUNICATION_ERROR => {
                        AuthenticationStatus::Unauthenticated(
                            UnauthenticatedReason::CommunicationError,
                        )
                    }
                    _ => {
                        error!("unknown unauthenticated reason");
                        return;
                    }
                }
            }
            _ => {
                error!("unknown authenticated state");
                return;
            }
        };

        debug!("Received connection status: {status:?}");

        if let Some(tx) = &client.tx_connection_status {
            tx.blocking_send(status)
                .expect("c_connection_status_callback: cannot blocking_send");
        }
    }

    unsafe extern "C" fn c_c2d_message_callback(
        handle: *mut IOTHUB_MESSAGE_HANDLE_DATA_TAG,
        context: *mut ::std::os::raw::c_void,
    ) -> IOTHUBMESSAGE_DISPOSITION_RESULT {
        let client = &mut *(context as *mut IotHubClient);

        if let Some(observer) = &client.tx_incoming_message {
            let mut property_keys: Vec<CString> = vec![];
            for property in &observer.properties {
                match CString::new(property.clone()) {
                    Ok(p) => property_keys.push(p),
                    Err(e) => {
                        error!(
                            "invalid property in c2d message received. payload: {property}, error: {e}"
                        );
                        return IOTHUBMESSAGE_DISPOSITION_RESULT_TAG_IOTHUBMESSAGE_REJECTED;
                    }
                }
            }

            match IotMessage::from_incoming_handle(handle, property_keys) {
                Ok(msg) => {
                    debug!("Received message from iothub: {msg:?}");

                    let (tx_result, rx_result) = oneshot::channel::<Result<DispositionResult>>();

                    observer
                        .observer
                        .blocking_send((msg, tx_result))
                        .expect("c_c2d_message_callback: cannot blocking_send");

                    match rx_result.blocking_recv() {
                        Ok(Ok(DispositionResult::Accepted)) => {
                            IOTHUBMESSAGE_DISPOSITION_RESULT_TAG_IOTHUBMESSAGE_ACCEPTED
                        }
                        Ok(Ok(DispositionResult::Rejected)) => {
                            IOTHUBMESSAGE_DISPOSITION_RESULT_TAG_IOTHUBMESSAGE_REJECTED
                        }
                        Ok(Ok(DispositionResult::Abandoned)) => {
                            IOTHUBMESSAGE_DISPOSITION_RESULT_TAG_IOTHUBMESSAGE_ABANDONED
                        }
                        Ok(Ok(DispositionResult::AsyncAck)) => {
                            IOTHUBMESSAGE_DISPOSITION_RESULT_TAG_IOTHUBMESSAGE_ASYNC_ACK
                        }
                        Ok(Err(e)) => {
                            error!("cannot handle c2d message: {e}");
                            IOTHUBMESSAGE_DISPOSITION_RESULT_TAG_IOTHUBMESSAGE_REJECTED
                        }
                        Err(e) => {
                            error!("channel unexpectedly closed: {e}");
                            IOTHUBMESSAGE_DISPOSITION_RESULT_TAG_IOTHUBMESSAGE_REJECTED
                        }
                    }
                }
                Err(e) => {
                    error!("cannot create IotMessage from incomming handle: {e}");
                    IOTHUBMESSAGE_DISPOSITION_RESULT_TAG_IOTHUBMESSAGE_REJECTED
                }
            }
        } else {
            match IotMessage::from_incoming_handle(handle, vec![]) {
                Ok(msg) => debug!("Received message from iothub: {msg:?}"),
                Err(e) => error!("cannot create IotMessage from incomming handle: {e}"),
            }

            IOTHUBMESSAGE_DISPOSITION_RESULT_TAG_IOTHUBMESSAGE_REJECTED
        }
    }

    unsafe extern "C" fn c_twin_callback(
        state: DEVICE_TWIN_UPDATE_STATE,
        payload: *const ::std::os::raw::c_uchar,
        size: usize,
        context: *mut ::std::os::raw::c_void,
    ) {
        match String::from_utf8(slice::from_raw_parts(payload, size).to_vec()) {
            Ok(desired_string) => {
                match serde_json::from_str::<serde_json::Value>(&desired_string) {
                    Ok(desired_json) => {
                        let client = &mut *(context as *mut IotHubClient);
                        let desired_state: TwinUpdateState = mem::transmute(state as i8);

                        debug!(
                            "Twin callback. state: {desired_state:?} size: {size} payload: {desired_json}"
                        );

                        if let Some(tx) = &client.tx_twin_desired {
                            tx.blocking_send((desired_state, desired_json))
                                .expect("c_twin_callback: cannot blocking_send");
                        }
                    }
                    Err(e) => error!(
                        "desired twin cannot be parsed. payload: {desired_string} error: {e}"
                    ),
                };
            }
            Err(e) => error!("desired twin cannot be parsed: {e}"),
        }
    }

    unsafe extern "C" fn c_reported_twin_callback(
        status_code: std::os::raw::c_int,
        context: *mut ::std::os::raw::c_void,
    ) {
        trace!("SendReportedTwin result: {status_code}");

        let result: Box<oneshot::Sender<bool>> =
            Box::from_raw(context as *mut oneshot::Sender<bool>);

        result
            .send(status_code == 204)
            .expect("c_reported_twin_callback: cannot send result");
    }

    unsafe extern "C" fn c_direct_method_callback(
        method_name: *const ::std::os::raw::c_char,
        payload: *const ::std::os::raw::c_uchar,
        size: usize,
        response: *mut *mut ::std::os::raw::c_uchar,
        response_size: *mut usize,
        context: *mut ::std::os::raw::c_void,
    ) -> ::std::os::raw::c_int {
        const METHOD_RESPONSE_SUCCESS: i32 = 200;
        const METHOD_RESPONSE_ERROR: i32 = 401;

        let empty_result: CString = CString::from_vec_unchecked(b"{ }".to_vec());
        *response_size = empty_result.as_bytes().len();
        *response = empty_result.into_raw() as *mut u8;

        let method_name = match CStr::from_ptr(method_name).to_str() {
            Ok(name) => name,
            Err(e) => {
                error!("cannot parse method name: {e}");
                return METHOD_RESPONSE_ERROR;
            }
        };

        let payload: serde_json::Value = match str::from_utf8(slice::from_raw_parts(payload, size))
        {
            Ok(p) => match serde_json::from_str(p) {
                Ok(json) => json,
                Err(e) => {
                    error!("cannot parse direct method payload: {e}");
                    return METHOD_RESPONSE_ERROR;
                }
            },
            Err(e) => {
                error!("cannot parse direct method payload: {e}");
                return METHOD_RESPONSE_ERROR;
            }
        };

        debug!("Received direct method call: {method_name:?} with payload: {payload}");

        let client = &mut *(context as *mut IotHubClient);

        if let Some(tx_direct_method) = &client.tx_direct_method {
            let (tx_result, rx_result) = oneshot::channel::<Result<Option<serde_json::Value>>>();

            tx_direct_method
                .blocking_send(DirectMethod {
                    name: method_name.to_string(),
                    payload,
                    responder: tx_result,
                })
                .expect("c_direct_method_callback: cannot blocking_send");

            match rx_result.blocking_recv() {
                Ok(Ok(None)) => {
                    debug!("direct method has no result");
                    return METHOD_RESPONSE_SUCCESS;
                }
                Ok(Ok(Some(result))) => {
                    debug!("direct method result: {result:?}");

                    match CString::new(result.to_string()) {
                        Ok(r) => {
                            *response_size = r.as_bytes().len();
                            *response = r.into_raw() as *mut u8;
                            return METHOD_RESPONSE_SUCCESS;
                        }
                        Err(e) => {
                            error!("cannot parse direct method result: {e}");
                        }
                    }
                }
                Ok(Err(e)) => {
                    error!("direct method error: {e:?}");

                    match CString::new(json!(e.to_string()).to_string()) {
                        Ok(r) => {
                            *response_size = r.as_bytes().len();
                            *response = r.into_raw() as *mut u8;
                        }
                        Err(e) => {
                            error!("cannot parse direct method result: {e}");
                        }
                    }
                }
                Err(e) => {
                    error!("channel unexpectedly closed: {e}");
                }
            }
        }

        METHOD_RESPONSE_ERROR
    }

    unsafe extern "C" fn c_d2c_confirmation_callback(
        status: IOTHUB_CLIENT_CONFIRMATION_RESULT,
        context: *mut std::ffi::c_void,
    ) {
        let result: Box<oneshot::Sender<bool>> =
            Box::from_raw(context as *mut oneshot::Sender<bool>);
        let mut succeeded = false;

        match status {
            IOTHUB_CLIENT_CONFIRMATION_RESULT_TAG_IOTHUB_CLIENT_CONFIRMATION_OK => {
                succeeded = true;
                debug!("c_d2c_confirmation_callback: received confirmation from iothub.");
            },
            IOTHUB_CLIENT_CONFIRMATION_RESULT_TAG_IOTHUB_CLIENT_CONFIRMATION_BECAUSE_DESTROY => error!("c_d2c_confirmation_callback: received confirmation from iothub with error IOTHUB_CLIENT_CONFIRMATION_BECAUSE_DESTROY."),
            IOTHUB_CLIENT_CONFIRMATION_RESULT_TAG_IOTHUB_CLIENT_CONFIRMATION_ERROR =>  error!("c_d2c_confirmation_callback: received confirmation from iothub with error IOTHUB_CLIENT_CONFIRMATION_ERROR."),
            IOTHUB_CLIENT_CONFIRMATION_RESULT_TAG_IOTHUB_CLIENT_CONFIRMATION_MESSAGE_TIMEOUT => error!("c_d2c_confirmation_callback: received confirmation from iothub with error IOTHUB_CLIENT_CONFIRMATION_MESSAGE_TIMEOUT."),
            _ => error!("c_d2c_confirmation_callback: received confirmation from iothub with unknown IOTHUB_CLIENT_CONFIRMATION_RESULT"),
        }

        result
            .send(succeeded)
            .expect("c_d2c_confirmation_callback: cannot send result");
    }

    fn spawn_confirmation(&mut self, rx: oneshot::Receiver<bool>) {
        let before = self.confirmation_set.len();
        let waker = task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut poll = Poll::Ready(Some(Ok::<_, JoinError>(())));

        // check if some confirmations run to completion meanwhile
        // we don't wait for completion here
        while let Poll::Ready(Some(Ok(()))) = poll {
            poll = self.confirmation_set.poll_join_next(&mut cx);
        }

        debug!(
            "cleaned {} confirmations",
            before - self.confirmation_set.len()
        );

        // spawn a task to handle the following results:
        //   - succeeded
        //   - failed
        //   - timed out
        self.confirmation_set.spawn(async move {
            match timeout(Duration::from_secs(Self::get_confirmation_timeout()), rx).await {
                // if really needed we could pass around the json of property or D2C msg to get logged here as context
                Ok(Ok(false)) => error!("confirmation failed"),
                Err(_) => warn!("confirmation timed out"),
                _ => debug!("confirmation successfully received"),
            }
        });
    }

    fn get_confirmation_timeout() -> u64 {
        static INIT: Once = Once::new();
        static mut CONFIRMATION_TIMEOUT_IN_SECS: u64 = CONFIRMATION_TIMEOUT_DEFAULT_IN_SECS;

        unsafe {
            INIT.call_once(|| {
                let mut confirmation_timeout_secs = None;

                if let Ok(timeout_secs) = env::var(AZURE_SDK_CONFIRMATION_TIMEOUT_IN_SECS) {
                    match timeout_secs.parse::<u64>() {
                        Ok(timeout_secs) => {
                            info!("set confirmation timeout to {timeout_secs}s");
                            confirmation_timeout_secs = Some(timeout_secs);
                        }
                        _ => error!("ignore invalid confirmation timeout {timeout_secs}"),
                    };
                }

                if confirmation_timeout_secs.is_none() {
                    confirmation_timeout_secs = Some(CONFIRMATION_TIMEOUT_DEFAULT_IN_SECS);
                    info!(
                        "set default confirmation timeout {CONFIRMATION_TIMEOUT_DEFAULT_IN_SECS}s"
                    )
                }
                CONFIRMATION_TIMEOUT_IN_SECS = confirmation_timeout_secs.unwrap()
            });
            CONFIRMATION_TIMEOUT_IN_SECS
        }
    }
}

impl Drop for IotHubClient {
    fn drop(&mut self) {
        self.twin.destroy()
    }
}
