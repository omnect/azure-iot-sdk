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
use anyhow::Result;
use azure_iot_sdk_sys::*;
use core::slice;
#[cfg(feature = "module_client")]
use eis_utils::*;
use futures::task;
use log::{debug, error, info, trace, warn};
use rand::Rng;
use serde_json::json;
use std::cell::RefCell;
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

static AZURE_SDK_LOGGING: &str = "AZURE_SDK_LOGGING";
static AZURE_SDK_DO_WORK_FREQUENCY_IN_MS: &str = "AZURE_SDK_DO_WORK_FREQUENCY_IN_MS";
static DO_WORK_FREQUENCY_RANGE_IN_MS: std::ops::RangeInclusive<u64> = 0..=100;
static DO_WORK_FREQUENCY_DEFAULT_IN_MS: u64 = 100;
static AZURE_SDK_CONFIRMATION_TIMEOUT_IN_SECS: &str = "AZURE_SDK_CONFIRMATION_TIMEOUT_IN_SECS";
static CONFIRMATION_TIMEOUT_DEFAULT_IN_SECS: u64 = 30;

#[cfg(feature = "module_client")]
macro_rules! days_to_secs {
    ($num_days:expr) => {
        $num_days * 24 * 60 * 60
    };
}

/// [Restart policy](https://github.com/Azure/azure-iot-sdk-c/blob/main/doc/connection_and_messaging_reliability.md#connection-retry-policies) used to connect to iot-hib
#[derive(Copy, Clone, Debug)]
pub enum RetryPolicy {
    /// check [here](https://github.com/Azure/azure-iot-sdk-c/blob/main/doc/connection_and_messaging_reliability.md#connection-retry-policies) for meaning
    None = 0,
    /// check [here](https://github.com/Azure/azure-iot-sdk-c/blob/main/doc/connection_and_messaging_reliability.md#connection-retry-policies) for meaning
    Immediate = 1,
    /// check [here](https://github.com/Azure/azure-iot-sdk-c/blob/main/doc/connection_and_messaging_reliability.md#connection-retry-policies) for meaning
    Interval = 2,
    /// check [here](https://github.com/Azure/azure-iot-sdk-c/blob/main/doc/connection_and_messaging_reliability.md#connection-retry-policies) for meaning
    LinearBackoff = 3,
    /// check [here](https://github.com/Azure/azure-iot-sdk-c/blob/main/doc/connection_and_messaging_reliability.md#connection-retry-policies) for meaning
    ExponentialBackoff = 4,
    /// check [here](https://github.com/Azure/azure-iot-sdk-c/blob/main/doc/connection_and_messaging_reliability.md#connection-retry-policies) for meaning
    ExponentialBackoffWithJitter = 5,
    /// check [here](https://github.com/Azure/azure-iot-sdk-c/blob/main/doc/connection_and_messaging_reliability.md#connection-retry-policies) for meaning
    Random = 6,
}

/// Indicates [type](https://docs.microsoft.com/en-us/azure/iot-hub/iot-hub-devguide-module-twins#back-end-operations) of desired properties update
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TwinUpdateState {
    /// complete update of desired properties
    Complete = 0,
    /// partial update of desired properties
    Partial = 1,
}

/// Used to update [desired properties](https://docs.microsoft.com/en-us/azure/iot-hub/iot-hub-devguide-module-twins#back-end-operations) to the client
pub struct TwinUpdate {
    /// type of update [`TwinUpdateState`]
    pub state: TwinUpdateState,
    /// value
    pub value: serde_json::Value,
}

/// Sender used to signal a new [`TwinUpdate`]
pub type TwinObserver = mpsc::Sender<TwinUpdate>;

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
    /// unknown
    Unknown,
}

/// Authentication status as a result of establishing a connection
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AuthenticationStatus {
    /// authenticated successfully
    Authenticated,
    /// authenticated not successfully with unauthenticated reason
    Unauthenticated(UnauthenticatedReason),
}

/// Sender used to signal a new [`AuthenticationStatus`]
pub type AuthenticationObserver = mpsc::Sender<AuthenticationStatus>;

/// DirectMethod
pub struct DirectMethod {
    /// method name
    pub name: String,
    /// method payload
    pub payload: serde_json::Value,
    /// method responder used by client to return the result
    pub responder: DirectMethodResponder,
}
/// Result used by iothub client consumer to send the result of a direct method
pub type DirectMethodResponder = oneshot::Sender<Result<Option<serde_json::Value>>>;
/// Sender used to signal a direct method to the iothub client consumer
pub type DirectMethodObserver = mpsc::Sender<DirectMethod>;

/// IncomingIotMessage
pub struct IncomingIotMessage {
    /// [`IotMessage`]
    pub inner: IotMessage,
    /// method responder used by client to return [`DispositionResult`]
    pub responder: DispositionResultResponder,
}
/// Result used by iothub client consumer to send the result of a direct method
pub type DispositionResultResponder = oneshot::Sender<Result<DispositionResult>>;
/// Sender used to signal a direct method to the iothub client consumer
pub type IotMessageSender = mpsc::Sender<IncomingIotMessage>;

/// Provides a channel and a property array to receive incoming cloud to device messages
#[derive(Clone, Debug)]
pub struct IncomingMessageObserver {
    responder: IotMessageSender,
    properties: Vec<String>,
}

impl IncomingMessageObserver {
    /// Creates a new instance of [`IncomingMessageObserver`]
    pub fn new(responder: IotMessageSender, properties: Vec<String>) -> Self {
        IncomingMessageObserver {
            responder,
            properties,
        }
    }
}

#[derive(Clone, Debug)]
struct RetrySetting {
    policy: RetryPolicy,
    timeout_secs: u32,
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
    tx_connection_status: Option<Box<AuthenticationObserver>>,
    tx_twin_desired: Option<Box<TwinObserver>>,
    tx_direct_method: Option<Box<DirectMethodObserver>>,
    tx_incoming_message: Option<Box<IncomingMessageObserver>>,
    model_id: Option<&'static str>,
    retry_setting: Option<RetrySetting>,
}

impl IotHubClientBuilder {
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
    pub fn build_edge_client(&self) -> Result<IotHubClient> {
        IotHubClient::from_edge_environment(self)
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
    pub fn build_device_client(&self, connection_string: &str) -> Result<IotHubClient> {
        IotHubClient::from_connection_string(connection_string, self)
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
    pub fn build_module_client(&self, connection_string: &str) -> Result<IotHubClient> {
        IotHubClient::from_connection_string(connection_string, self)
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
    pub async fn build_module_client_from_identity(&self) -> Result<IotHubClient> {
        IotHubClient::from_identity_service(self).await
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
    pub fn observe_connection_state(
        mut self,
        tx_connection_status: AuthenticationObserver,
    ) -> Self {
        self.tx_connection_status = Some(Box::new(tx_connection_status));
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
    pub fn observe_desired_properties(mut self, tx_twin_desired: TwinObserver) -> Self {
        self.tx_twin_desired = Some(Box::new(tx_twin_desired));
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
    pub fn observe_direct_methods(mut self, tx_direct_method: DirectMethodObserver) -> Self {
        self.tx_direct_method = Some(Box::new(tx_direct_method));
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
    pub fn observe_incoming_messages(
        mut self,
        tx_incoming_message: IncomingMessageObserver,
    ) -> Self {
        self.tx_incoming_message = Some(Box::new(tx_incoming_message));
        self
    }

    /// Set an Azure IoT Plug & Play model id.
    /// ```no_run
    /// use azure_iot_sdk::client::*;
    /// use std::{thread, time};
    /// use tokio::{select, sync::mpsc};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     #[cfg(feature = "edge_client")]
    ///     let mut client = IotHubClient::builder()
    ///         .pnp_model_id("my.pnp.id")
    ///         .build_edge_client()
    ///         .unwrap();
    ///     #[cfg(feature = "device_client")]
    ///     let mut client = IotHubClient::builder()
    ///         .pnp_model_id("my.pnp.id")
    ///         .build_device_client("my-connection-string")
    ///         .unwrap();
    ///     #[cfg(feature = "module_client")]
    ///     let mut client = IotHubClient::builder()
    ///         .pnp_model_id("my.pnp.id")
    ///         .build_module_client("my-connection-string")
    ///         .unwrap();
    /// }
    /// ```
    pub fn pnp_model_id(mut self, model_id: &'static str) -> Self {
        self.model_id = Some(model_id);
        self
    }

    /// Call this function to set the restart policy used for connecting to iot-hub.
    /// ```no_run
    /// use azure_iot_sdk::client::*;
    /// use std::{thread, time};
    /// use tokio::{select, sync::mpsc};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     #[cfg(feature = "edge_client")]
    ///     let mut client = IotHubClient::builder()
    ///         .retry_policy(RetryPolicy::None, 0)
    ///         .build_edge_client()
    ///         .unwrap();
    ///     #[cfg(feature = "device_client")]
    ///     let mut client = IotHubClient::builder()
    ///         .retry_policy(RetryPolicy::None, 0)
    ///         .build_device_client("my-connection-string")
    ///         .unwrap();
    ///     #[cfg(feature = "module_client")]
    ///     let mut client = IotHubClient::builder()
    ///         .retry_policy(RetryPolicy::None, 0)
    ///         .build_module_client("my-connection-string")
    ///         .unwrap();
    /// }
    /// ```
    pub fn retry_policy(mut self, policy: RetryPolicy, timeout_secs: u32) -> Self {
        self.retry_setting = Some(RetrySetting {
            policy,
            timeout_secs,
        });
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
    tx_connection_status: Option<Box<AuthenticationObserver>>,
    tx_twin_desired: Option<Box<TwinObserver>>,
    tx_direct_method: Option<Box<DirectMethodObserver>>,
    tx_incoming_message: Option<Box<IncomingMessageObserver>>,
    model_id: Option<&'static str>,
    retry_setting: Option<RetrySetting>,
    confirmation_set: RefCell<JoinSet<()>>,
}

impl IotHubClient {
    /// Call this function in order to get the underlying azure-sdk-c version string.
    /// ```rust, no_run
    /// use azure_iot_sdk::client::*;
    ///
    /// IotHubClient::sdk_version_string();
    /// ```
    pub fn sdk_version_string() -> String {
        twin::sdk_version_string()
    }

    /// Call this function to get the configured [`ClientType`].
    /// ```rust, no_run
    /// use azure_iot_sdk::client::*;
    ///
    /// IotHubClient::client_type();
    /// ```
    pub fn client_type() -> ClientType {
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

    /// Call this function to get a builder to build an instance of [`IotHubClient`].
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
    /// }
    /// ```
    pub fn builder() -> IotHubClientBuilder {
        IotHubClientBuilder::default()
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
    pub fn send_d2c_message(&self, mut message: IotMessage) -> Result<()> {
        let trace_id: u32 = rand::thread_rng().gen();
        let handle = message.create_outgoing_handle()?;
        let queue = message.output_queue.clone();
        let (tx, rx) = oneshot::channel::<bool>();

        debug!("send_d2c_message({trace_id}): {queue:?}");

        self.twin.send_event_to_output_async(
            handle,
            queue,
            Some(IotHubClient::c_d2c_confirmation_callback),
            Box::into_raw(Box::new((tx, trace_id))) as *mut c_void,
        )?;

        self.spawn_confirmation((rx, trace_id));

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
    pub fn twin_report(&self, reported: serde_json::Value) -> Result<()> {
        let trace_id: u32 = rand::thread_rng().gen();
        debug!("send reported({trace_id}): {reported:?}");

        let reported_state = CString::new(reported.to_string())?;
        let size = reported_state.as_bytes().len();
        let (tx, rx) = oneshot::channel::<bool>();

        self.twin.send_reported_state(
            reported_state,
            size,
            Some(IotHubClient::c_reported_twin_callback),
            Box::into_raw(Box::new((tx, trace_id))) as *mut c_void,
        )?;

        self.spawn_confirmation((rx, trace_id));

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
    pub fn twin_async(&mut self) -> Result<()> {
        debug!("twin_complete: get entire twin");

        let Some(tx) = self.tx_twin_desired.as_deref_mut() else {
            anyhow::bail!("twin observer not present")
        };

        self.twin.twin_async(
            Some(IotHubClient::c_twin_callback),
            tx as *mut TwinObserver as *mut c_void,
        )
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
    #[allow(clippy::await_holding_refcell_ref)]
    pub async fn shutdown(&self) {
        info!("shutdown");

        let join_all = async {
            debug!(
                "there are {} pending confirmations.",
                self.confirmation_set.borrow().len()
            );
            while self
                .confirmation_set
                .borrow_mut()
                .join_next()
                .await
                .is_some()
            {}
        };

        if tokio::time::timeout(
            Duration::from_secs(Self::get_confirmation_timeout()),
            join_all,
        )
        .await
        .is_err()
        {
            warn!(
                "there are {} pending confirmations on shutdown.",
                self.confirmation_set.borrow().len()
            );
        }

        /*
           We abort and join all "wait for pending confirmations" tasks
           (https://docs.rs/tokio/latest/src/tokio/task/join_set.rs.html#362).
           This means:
               - we have a clean shutdown
               - we shutdown as fast as possible
               - we DON'T WAIT for pending confirmations

            Further the following line of code creates a false positive clippy warning which we
            allow on function scope.
        */

        self.confirmation_set.borrow_mut().shutdown().await;
    }

    #[cfg(feature = "edge_client")]
    pub(crate) fn from_edge_environment(params: &IotHubClientBuilder) -> Result<IotHubClient> {
        IotHubClient::iothub_init()?;

        let mut twin = Box::<ModuleTwin>::default();

        twin.create_from_edge_environment()?;

        let mut client = IotHubClient {
            twin,
            tx_connection_status: params.tx_connection_status.clone(),
            tx_twin_desired: params.tx_twin_desired.clone(),
            tx_direct_method: params.tx_direct_method.clone(),
            tx_incoming_message: params.tx_incoming_message.clone(),
            model_id: params.model_id,
            retry_setting: params.retry_setting.clone(),
            confirmation_set: JoinSet::new().into(),
        };

        client.set_callbacks()?;

        client.set_options()?;

        Ok(client)
    }

    #[cfg(feature = "module_client")]
    pub(crate) async fn from_identity_service(params: &IotHubClientBuilder) -> Result<Self> {
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

        IotHubClient::from_connection_string(connection_info.connection_string.as_str(), params)
    }

    #[cfg(any(feature = "module_client", feature = "device_client"))]
    pub(crate) fn from_connection_string(
        connection_string: &str,
        params: &IotHubClientBuilder,
    ) -> Result<Self> {
        IotHubClient::iothub_init()?;

        #[cfg(feature = "module_client")]
        let mut twin = Box::<ModuleTwin>::default();

        #[cfg(feature = "device_client")]
        let mut twin = Box::<DeviceTwin>::default();

        twin.create_from_connection_string(CString::new(connection_string)?)?;

        let mut client = IotHubClient {
            twin,
            tx_connection_status: params.tx_connection_status.clone(),
            tx_twin_desired: params.tx_twin_desired.clone(),
            tx_direct_method: params.tx_direct_method.clone(),
            tx_incoming_message: params.tx_incoming_message.clone(),
            model_id: params.model_id,
            retry_setting: params.retry_setting.clone(),
            confirmation_set: JoinSet::new().into(),
        };

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
        if let Some(tx) = self.tx_connection_status.as_deref_mut() {
            self.twin.set_connection_status_callback(
                Some(IotHubClient::c_connection_status_callback),
                tx as *mut AuthenticationObserver as *mut c_void,
            )?;
        }

        if let Some(tx) = self.tx_incoming_message.as_deref_mut() {
            self.twin.set_input_message_callback(
                Some(IotHubClient::c_c2d_message_callback),
                tx as *mut IncomingMessageObserver as *mut c_void,
            )?;
        }

        if let Some(tx) = self.tx_twin_desired.as_deref_mut() {
            self.twin.set_twin_callback(
                Some(IotHubClient::c_twin_callback),
                tx as *mut TwinObserver as *mut c_void,
            )?;
        }

        if let Some(tx) = self.tx_direct_method.as_deref_mut() {
            self.twin.set_method_callback(
                Some(IotHubClient::c_direct_method_callback),
                tx as *mut DirectMethodObserver as *mut c_void,
            )?;
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

        if let Some(model_id) = self.model_id {
            info!("set pnp model id: {model_id}");
            let model_id = CString::new(model_id)?;
            self.twin.set_option(
                CString::new("model_id")?,
                model_id.as_ptr() as *const c_void,
            )?;
        }

        if let Some(retry_setting) = &self.retry_setting {
            info!("set retry policy: {retry_setting:?}");
            self.twin.set_retry_policy(
                retry_setting.policy as u32,
                retry_setting.timeout_secs as usize,
            )?;
        }

        Ok(())
    }

    unsafe extern "C" fn c_connection_status_callback(
        connection_status: IOTHUB_CLIENT_CONNECTION_STATUS,
        status_reason: IOTHUB_CLIENT_CONNECTION_STATUS_REASON,
        context: *mut ::std::os::raw::c_void,
    ) {
        let tx = &mut *(context as *mut AuthenticationObserver);

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

                        AuthenticationStatus::Unauthenticated(UnauthenticatedReason::Unknown)
                    }
                }
            }
            _ => {
                error!("unknown authenticated state");
                return;
            }
        };

        debug!("Received connection status: {status:?}");

        tx.blocking_send(status)
            .expect("c_connection_status_callback: cannot blocking_send");
    }

    unsafe extern "C" fn c_c2d_message_callback(
        handle: *mut IOTHUB_MESSAGE_HANDLE_DATA_TAG,
        context: *mut ::std::os::raw::c_void,
    ) -> IOTHUBMESSAGE_DISPOSITION_RESULT {
        let observer = &mut *(context as *mut IncomingMessageObserver);
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
                    .responder
                    .blocking_send(IncomingIotMessage {
                        inner: msg,
                        responder: tx_result,
                    })
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
                        error!("c2d msg result channel unexpectedly closed: {e}");
                        IOTHUBMESSAGE_DISPOSITION_RESULT_TAG_IOTHUBMESSAGE_REJECTED
                    }
                }
            }
            Err(e) => {
                error!("cannot create IotMessage from incomming handle: {e}");
                IOTHUBMESSAGE_DISPOSITION_RESULT_TAG_IOTHUBMESSAGE_REJECTED
            }
        }
    }

    unsafe extern "C" fn c_twin_callback(
        state: DEVICE_TWIN_UPDATE_STATE,
        payload: *const ::std::os::raw::c_uchar,
        size: usize,
        context: *mut ::std::os::raw::c_void,
    ) {
        let tx = &mut *(context as *mut TwinObserver);

        match String::from_utf8(slice::from_raw_parts(payload, size).to_vec()) {
            Ok(desired_string) => {
                match serde_json::from_str::<serde_json::Value>(&desired_string) {
                    Ok(desired_json) => {
                        let desired_state: TwinUpdateState = mem::transmute(state as i8);

                        debug!(
                            "Twin callback. state: {desired_state:?} size: {size} payload: {desired_json}"
                        );

                        tx.blocking_send(TwinUpdate {
                            state: desired_state,
                            value: desired_json,
                        })
                        .expect("c_twin_callback: cannot blocking_send");
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

        let (tx_confirm, trace_id) = *Box::from_raw(context as *mut (oneshot::Sender<bool>, u32));

        if tx_confirm.send(status_code == 204).is_err() {
            error!("c_reported_twin_callback({trace_id}): cannot send result {status_code} for confirmation since receiver already timed out and dropped ");
        }
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

        let tx_direct_method = &mut *(context as *mut DirectMethodObserver);

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
                error!("direct method result channel unexpectedly closed: {e}");
            }
        }

        METHOD_RESPONSE_ERROR
    }

    unsafe extern "C" fn c_d2c_confirmation_callback(
        status: IOTHUB_CLIENT_CONFIRMATION_RESULT,
        context: *mut std::ffi::c_void,
    ) {
        let (tx_confirm, trace_id) = *Box::from_raw(context as *mut (oneshot::Sender<bool>, u32));
        let mut succeeded = false;

        match status {
            IOTHUB_CLIENT_CONFIRMATION_RESULT_TAG_IOTHUB_CLIENT_CONFIRMATION_OK => {
                succeeded = true;
                debug!("c_d2c_confirmation_callback({trace_id}): received confirmation from iothub.");
            },
            IOTHUB_CLIENT_CONFIRMATION_RESULT_TAG_IOTHUB_CLIENT_CONFIRMATION_BECAUSE_DESTROY => error!("c_d2c_confirmation_callback ({trace_id}): received confirmation from iothub with error IOTHUB_CLIENT_CONFIRMATION_BECAUSE_DESTROY."),
            IOTHUB_CLIENT_CONFIRMATION_RESULT_TAG_IOTHUB_CLIENT_CONFIRMATION_ERROR =>  error!("c_d2c_confirmation_callback ({trace_id}): received confirmation from iothub with error IOTHUB_CLIENT_CONFIRMATION_ERROR."),
            IOTHUB_CLIENT_CONFIRMATION_RESULT_TAG_IOTHUB_CLIENT_CONFIRMATION_MESSAGE_TIMEOUT => error!("c_d2c_confirmation_callback ({trace_id}): received confirmation from iothub with error IOTHUB_CLIENT_CONFIRMATION_MESSAGE_TIMEOUT."),
            _ => error!("c_d2c_confirmation_callback({trace_id}): received confirmation from iothub with unknown IOTHUB_CLIENT_CONFIRMATION_RESULT"),
        }

        tx_confirm.send(succeeded).expect(&format!(
            "c_d2c_confirmation_callback({trace_id}): cannot send confirmation result"
        ));
    }

    fn spawn_confirmation(&self, (rx, trace_id): (oneshot::Receiver<bool>, u32)) {
        let before = self.confirmation_set.borrow().len();
        let waker = task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut poll = Poll::Ready(Some(Ok::<_, JoinError>(())));

        // check if some confirmations run to completion meanwhile
        // we don't wait for completion here
        while let Poll::Ready(Some(Ok(()))) = poll {
            poll = self.confirmation_set.borrow_mut().poll_join_next(&mut cx);
        }

        trace!(
            "cleaned {} confirmations",
            before - self.confirmation_set.borrow().len()
        );

        // spawn a task to wait for confirmation and handle the following results:
        //   - succeeded: confirmation callback sent success
        //   - failed: confirmation callback sent failure
        //   - timed out: confirmation didn't send anything
        self.confirmation_set.borrow_mut().spawn(async move {
            match timeout(Duration::from_secs(Self::get_confirmation_timeout()), rx).await {
                // if really needed we could pass around the json of property or D2C msg to get logged here as context
                Ok(Ok(false)) => error!("confirmation({trace_id}): failed"),
                Err(_) => warn!("confirmation({trace_id}): timed out"),
                _ => debug!("confirmation({trace_id}): successfully received"),
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
