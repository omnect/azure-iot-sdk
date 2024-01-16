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
use async_trait::async_trait;
use azure_iot_sdk_sys::*;
use core::slice;
#[cfg(any(feature = "module_client", feature = "device_client"))]
use eis_utils::*;
use futures::task;
use log::{debug, error, trace};
use serde_json::json;
#[cfg(any(feature = "module_client", feature = "device_client"))]
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

/// Result used by iothub client consumer to send the result of a direct method
pub type DirectMethodResult = oneshot::Sender<Result<Option<serde_json::Value>>>;
/// Sender used to signal a direct method to the iothub client consumer
pub type DirectMethodSender = mpsc::Sender<(String, serde_json::Value, DirectMethodResult)>;

/// Trait which provides functions for communication with azure iothub
#[async_trait(?Send)]
pub trait IotHub {
    /// Call this function in order to get the underlying azure-sdk-c version string.
    fn sdk_version_string() -> String
    where
        Self: Sized;

    /// Call this function to get the configured [`ClientType`].
    fn client_type() -> ClientType
    where
        Self: Sized;

    /// Call this function in order to create an instance of [`IotHubClient`] from iotedge environment.<br>
    /// ***Note***: this feature is only available with "edge_client" feature enabled.
    fn from_edge_environment(
        tx_connection_status: Option<mpsc::Sender<AuthenticationStatus>>,
        tx_twin_desired: Option<mpsc::Sender<(TwinUpdateState, serde_json::Value)>>,
        tx_direct_method: Option<DirectMethodSender>,
        tx_incoming_message: Option<IncomingMessageObserver>,
    ) -> Result<Box<Self>>
    where
        Self: Sized;

    /// Call this function in order to create an instance of [`IotHubClient`] by using iot-identity-service.<br>
    /// ***Note1***: this feature is only available with "device_client" or "module_client" feature enabled.<br>
    /// ***Note2***: this feature is currently [not supported for all combinations of identity type and authentication mechanism](https://azure.github.io/iot-identity-service/develop-an-agent.html#connecting-your-agent-to-iot-hub).
    async fn from_identity_service(
        _tx_connection_status: Option<mpsc::Sender<AuthenticationStatus>>,
        _tx_twin_desired: Option<mpsc::Sender<(TwinUpdateState, serde_json::Value)>>,
        _tx_direct_method: Option<DirectMethodSender>,
        _tx_incoming_message: Option<IncomingMessageObserver>,
    ) -> Result<Box<Self>>
    where
        Self: Sized;

    /// Call this function in order to create an instance of [`IotHubClient`] by using a connection string.
    fn from_connection_string(
        connection_string: &str,
        tx_connection_status: Option<mpsc::Sender<AuthenticationStatus>>,
        tx_twin_desired: Option<mpsc::Sender<(TwinUpdateState, serde_json::Value)>>,
        tx_direct_method: Option<DirectMethodSender>,
        tx_incoming_message: Option<IncomingMessageObserver>,
    ) -> Result<Box<Self>>
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

/// iothub cloud to device (C2D) and device to cloud (D2C) messages
mod message;
/// client implementation, either device, module or edge
mod twin;

static mut IOTHUB_INIT_RESULT: i32 = -1;
static IOTHUB_INIT_ONCE: Once = Once::new();
static DO_WORK_FREQUENCY_IN_MS: &str = "DO_WORK_FREQUENCY_IN_MS";
static DO_WORK_FREQUENCY_RANGE_IN_MS: std::ops::RangeInclusive<u64> = 0..=100;

#[cfg(any(feature = "module_client", feature = "device_client"))]
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
#[derive(Debug)]
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
///
///     let mut client = IotHubClient::from_identity_service(
///         Some(tx_connection_status),
///         Some(tx_twin_desired),
///         Some(tx_direct_method),
///         Some(IncomingMessageObserver::new(tx_incoming_message, vec![])),
///     )
///     .await
///     .unwrap();
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

    /// Call this function in order to create an instance of [`IotHubClient`] from iotedge environment.<br>
    /// ***Note***: this feature is only available with "edge_client" feature enabled.
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
    ///     let mut client = IotHubClient::from_edge_environment(
    ///         Some(tx_connection_status),
    ///         Some(tx_twin_desired),
    ///         Some(tx_direct_method),
    ///         Some(IncomingMessageObserver::new(tx_incoming_message, vec![])),
    ///     )
    ///     .unwrap();
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
    #[allow(unused)]
    fn from_edge_environment(
        tx_connection_status: Option<mpsc::Sender<AuthenticationStatus>>,
        tx_twin_desired: Option<mpsc::Sender<(TwinUpdateState, serde_json::Value)>>,
        tx_direct_method: Option<DirectMethodSender>,
        tx_incoming_message: Option<IncomingMessageObserver>,
    ) -> Result<Box<Self>> {
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

    /// Call this function in order to create an instance of [`IotHubClient`] by using iot-identity-service.<br>
    /// ***Note1***: this feature is only available with "device_client" or "module_client" feature enabled.<br>
    /// ***Note2***: this feature is currently [not supported for all combinations of identity type and authentication mechanism](https://azure.github.io/iot-identity-service/develop-an-agent.html#connecting-your-agent-to-iot-hub).
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
    ///     let mut client = IotHubClient::from_identity_service(
    ///         Some(tx_connection_status),
    ///         Some(tx_twin_desired),
    ///         Some(tx_direct_method),
    ///         Some(IncomingMessageObserver::new(tx_incoming_message, vec![])),
    ///     )
    ///     .await
    ///     .unwrap();
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
    async fn from_identity_service(
        _tx_connection_status: Option<mpsc::Sender<AuthenticationStatus>>,
        _tx_twin_desired: Option<mpsc::Sender<(TwinUpdateState, serde_json::Value)>>,
        _tx_direct_method: Option<DirectMethodSender>,
        _tx_incoming_message: Option<IncomingMessageObserver>,
    ) -> Result<Box<Self>> {
        #[cfg(feature = "edge_client")]
        anyhow::bail!(
            "edge modules can't use from_identity_service to get connection string. either use from_edge_environment() or from_connection_string().",
        );

        #[cfg(any(feature = "module_client", feature = "device_client"))]
        {
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
    }

    /// Call this function in order to create an instance of [`IotHubClient`] by using a connection string.
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
    ///     let mut client = IotHubClient::from_connection_string(
    ///         "my connection string",
    ///         Some(tx_connection_status),
    ///         Some(tx_twin_desired),
    ///         Some(tx_direct_method),
    ///         Some(IncomingMessageObserver::new(tx_incoming_message, vec![])),
    ///     )
    ///     .unwrap();
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
    fn from_connection_string(
        connection_string: &str,
        tx_connection_status: Option<mpsc::Sender<AuthenticationStatus>>,
        tx_twin_desired: Option<mpsc::Sender<(TwinUpdateState, serde_json::Value)>>,
        tx_direct_method: Option<DirectMethodSender>,
        tx_incoming_message: Option<IncomingMessageObserver>,
    ) -> Result<Box<Self>> {
        IotHubClient::iothub_init()?;

        #[cfg(any(feature = "module_client", feature = "edge_client"))]
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

    /// Call this function to send a message (D2C) to iothub.
    /// ```rust, no_run
    /// use azure_iot_sdk::client::*;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let mut client = IotHubClient::from_identity_service(None, None, None, None).await.unwrap();
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
    ///     let mut client = IotHubClient::from_identity_service(None, None, None, None).await.unwrap();
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
    ///     let mut client = IotHubClient::from_identity_service(None, None, None, None).await.unwrap();
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
    ///     let mut client = IotHubClient::from_identity_service(None, None, None, None).await.unwrap();
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
        let mut poll = Some(Ok::<_, JoinError>(()));

        // join shouldn't take much longer than CONFIRMATION_TIMEOUT_SECS
        while poll.is_some() {
            poll = self.confirmation_set.join_next().await;
        }
    }
}

impl IotHubClient {
    const CONFIRMATION_TIMEOUT_SECS: u64 = 5;

    fn iothub_init() -> Result<()> {
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
        if let Ok(freq) = env::var(DO_WORK_FREQUENCY_IN_MS) {
            match freq.parse::<u64>() {
                Ok(freq) if DO_WORK_FREQUENCY_RANGE_IN_MS.contains(&freq) => {
                    let mut mut_freq = freq;
                    debug!("set do_work frequency to {mut_freq}ms");
                    self.twin.set_option(
                        CString::new("do_work_freq_ms")?,
                        &mut mut_freq as *mut uint_fast64_t as *mut c_void,
                    )?;
                }
                _ => error!("ignore do_work frequency {freq} since not in range of {DO_WORK_FREQUENCY_RANGE_IN_MS:?}ms"),
            };
        }

        if cfg!(feature = "iot_c_sdk_logs") {
            self.twin.set_option(
                CString::new("logtrace")?,
                &mut true as *mut bool as *mut c_void,
            )?
        }
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
                .blocking_send((method_name.to_string(), payload, tx_result))
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

        trace!(
            "cleaned {} confirmations",
            before - self.confirmation_set.len()
        );

        // spawn a task to handle the following results:
        //   - succeeded
        //   - failed
        //   - timed out
        self.confirmation_set.spawn(async move {
            match timeout(Duration::from_secs(Self::CONFIRMATION_TIMEOUT_SECS), rx).await {
                Ok(Ok(false)) => panic!("twin_report: failed"),
                Err(_) => panic!("twin_report: timed out"),
                _ => trace!("twin_report: succeeded"),
            }
        });
    }
}

impl Drop for IotHubClient {
    fn drop(&mut self) {
        self.twin.destroy()
    }
}
