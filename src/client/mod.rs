//! Let's you create an instance of [`Self::IotHubClient`] with integrated [`Self::EventHandler`].
pub use self::message::{Direction, IotMessage, IotMessageBuilder};
pub use self::twin::TwinType;
use self::twin::{DeviceTwin, ModuleTwin, Twin};
use azure_iot_sdk_sys::*;
use core::slice;
use eis_utils::*;
use log::{debug, error};
use rand::Rng;
use serde_json::json;
use std::boxed::Box;
use std::collections::HashMap;
use std::error::Error;
use std::ffi::{c_void, CStr, CString};
use std::mem;
use std::str;
use std::sync::Once;
use std::time::{Duration, SystemTime};

/// crate wide shortcut for error type
pub type IotError = Box<dyn Error + Send + Sync>;

/// iothub cloud to device (C2D) and device to cloud (D2C) messages
mod message;
/// twin implementation, ether device or module twin
mod twin;

static mut IOTHUB_INIT_RESULT: i32 = -1;
static IOTHUB_INIT_ONCE: Once = Once::new();

macro_rules! days_to_secs {
    ($num_days:expr) => {
        $num_days * 24 * 60 * 60
    };
}

/// indicates [type](https://docs.microsoft.com/en-us/azure/iot-hub/iot-hub-devguide-module-twins#back-end-operations) of desired properties update
#[derive(Debug)]
pub enum TwinUpdateState {
    /// complete update of desired properties
    Complete = 0,
    /// partial update of desired properties
    Partial = 1,
}

/// reason for unauthenticated connection result
#[derive(Debug)]
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

/// authentication status as a result of establishing a connection
#[derive(Debug)]
pub enum AuthenticationStatus {
    /// authenticated successfully
    Authenticated,
    /// authenticated not successfully with unauthenticated reason
    Unauthenticated(UnauthenticatedReason),
}

/// function signature definition for a direct method implementation
/// ```no_run
/// # use azure_iot_sdk::client::*;
/// # use serde_json::json;
/// #
/// fn mirror_func_params_as_result(
///     in_json: serde_json::Value,
/// ) -> Result<Option<serde_json::Value>, IotError> {
///     let out_json = json!({
///         "called function": "func_params_as_result",
///         "your param was": in_json
///     });
///     Ok(Some(out_json))
/// }
///
/// fn main() {
///     let dm1 = Box::new(mirror_func_params_as_result);
///     let dm2 = IotHubClient::make_direct_method(move |_in_json| Ok(None));
/// }
/// ```
pub type DirectMethod =
    Box<(dyn Fn(serde_json::Value) -> Result<Option<serde_json::Value>, IotError> + Send)>;

/// hash map signature definition for a direct method map implementation
/// ```no_run
/// # use azure_iot_sdk::client::*;
/// # use serde_json::json;
/// #
/// fn mirror_func_params_as_result(
///     in_json: serde_json::Value,
/// ) -> Result<Option<serde_json::Value>, IotError> {
///     let out_json = json!({
///         "called function": "func_params_as_result",
///         "your param was": in_json
///     });
///     Ok(Some(out_json))
/// }
///
/// fn main() {
///     let dm1 = Box::new(mirror_func_params_as_result);
///     let dm2 = IotHubClient::make_direct_method(move |_in_json| Ok(None));
///
///     let mut dmm = DirectMethodMap::new();
///
///     dmm.insert(String::from("mirror_func_params_as_result"), dm1);
///     dmm.insert(String::from("closure"), dm2);
/// }
/// ```
pub type DirectMethodMap = HashMap<String, DirectMethod>;

/// Trait to be implemented by client implementations in order to handle iothub events
///  ```no_run
/// # use azure_iot_sdk::client::*;
/// # use log::debug;
/// #
/// struct MyEventHandler {}
///
/// impl EventHandler for MyEventHandler {
///     fn handle_connection_status(&self, auth_status: AuthenticationStatus) {
///         debug!("{:?}", auth_status)
///     }
///
///     fn handle_c2d_message(&self, message: IotMessage) -> Result<(), IotError> {
///         debug!("{:?}", message);
///         Ok(())
///     }
///
///     fn get_c2d_message_property_keys(&self) -> Vec<&'static str> {
///         debug!("get message properties called");
///         vec![]
///     }
///
///     fn handle_twin_desired(
///         &self,
///         state: TwinUpdateState,
///         desired: serde_json::Value,
///     ) -> Result<(), IotError> {
///         debug!("{:?}, {:?}", state, desired);
///         Ok(())
///     }
///
///     fn get_direct_methods(&self) -> Option<&DirectMethodMap> {
///         debug!("get direct methods called");
///         None
///     }
/// }
/// ```
pub trait EventHandler {
    /// gets called as a result of calling [`IotHubClient::from_identity_service()`] or
    /// [`IotHubClient::from_connection_string()`]
    fn handle_connection_status(&self, auth_status: AuthenticationStatus) {
        debug!(
            "unhandled call to handle_connection_status(). status: {:?}",
            auth_status
        )
    }

    /// gets called if a message from iothub is received
    fn handle_c2d_message(&self, message: IotMessage) -> Result<(), IotError> {
        debug!("unhandled call to handle_message(). message: {:?}", message);
        Ok(())
    }

    /// gets called in order to return [mqtt property keys](https://docs.microsoft.com/de-de/azure/iot-hub/iot-c-sdk-ref/iothub-message-h/iothubmessage-getproperty) to extracted from
    /// an iothub message
    fn get_c2d_message_property_keys(&self) -> Vec<&'static str> {
        vec![]
    }

    /// gets called if new desired properties were sent by iothub
    fn handle_twin_desired(
        &self,
        state: TwinUpdateState,
        desired: serde_json::Value,
    ) -> Result<(), IotError> {
        debug!(
            "unhandled call to handle_twin_desired(). state: {:?} twin: {}",
            state,
            desired.to_string()
        );
        Ok(())
    }

    /// gets called if a direct method was called by iothub.
    /// client implementation must return a map with implemented direct methods.
    fn get_direct_methods(&self) -> Option<&DirectMethodMap> {
        debug!("unhandled call to get_direct_methods().");
        None
    }
}

/// iothub client to be instantiated in order to initiate iothub communication
/// ```no_run
/// # use azure_iot_sdk::client::*;
/// # use log::debug;
/// # use std::{thread, time};
/// #
/// struct MyEventHandler {}
///
/// impl EventHandler for MyEventHandler {
///     fn handle_connection_status(&self, auth_status: AuthenticationStatus) {
///         debug!("{:?}", auth_status)
///     }
///
///     fn handle_c2d_message(&self, message: IotMessage) -> Result<(), IotError> {
///         debug!("{:?}", message);
///         Ok(())
///     }
///
///     fn get_c2d_message_property_keys(&self) -> Vec<&'static str> {
///         debug!("get message properties called");
///         vec![]
///     }
///
///     fn handle_twin_desired(
///         &self,
///         state: TwinUpdateState,
///         desired: serde_json::Value,
///     ) -> Result<(), IotError> {
///         debug!("{:?}, {:?}", state, desired);
///         Ok(())
///     }
///
///     fn get_direct_methods(&self) -> Option<&DirectMethodMap> {
///         debug!("get direct methods called");
///         None
///     }
/// }
///
/// fn main() {
///     let event_handler = MyEventHandler{};
///     let mut client = IotHubClient::from_identity_service(TwinType::Module, event_handler).unwrap();
///
///     loop {
///         client.do_work();
///         thread::sleep(time::Duration::from_millis(100));
///     }
/// }
/// ```
pub struct IotHubClient {
    twin: Box<dyn Twin>,
    event_handler: Box<dyn EventHandler>,
}

impl IotHubClient {
    /// call this function in order to create an instance of [`IotHubClient`] by using iot-identity-service.
    /// ***Note***: this feature is currently [not supported for all combinations of identity type and authentication mechanism](https://azure.github.io/iot-identity-service/develop-an-agent.html#connecting-your-agent-to-iot-hub).
    /// ```rust, no_run
    /// # use azure_iot_sdk::client::*;
    /// # struct MyEventHandler {}
    /// # impl EventHandler for MyEventHandler {}
    /// #
    /// let event_handler = MyEventHandler{};
    /// let mut client = IotHubClient::from_identity_service(TwinType::Module, event_handler).unwrap();
    /// ```
    pub fn from_identity_service(
        twin_type: TwinType,
        event_handler: impl EventHandler + 'static,
    ) -> Result<Box<Self>, IotError> {
        let connection_info = request_connection_string_from_eis_with_expiry(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)?
                .saturating_add(Duration::from_secs(days_to_secs!(30))),
        )
        .map_err(|err| {
            if let TwinType::Device = twin_type {
                error!("iot identity service failed to create device twin identity.
                In case you use TPM attestation please note that this combination is currently not supported.");
            }

            err
        })?;

        IotHubClient::from_connection_string(
            twin_type,
            connection_info.connection_string.as_str(),
            event_handler,
        )
    }

    /// call this function in order to create an instance of [`IotHubClient`] by using a connection string.
    /// ```rust, no_run
    /// # use azure_iot_sdk::client::*;
    /// # struct MyEventHandler {}
    /// # impl EventHandler for MyEventHandler {}
    /// #
    /// let event_handler = MyEventHandler{};
    /// let con_str = "HostName=my-iothub.azure-devices.net;DeviceId=my-dev-id;SharedAccessKey=my-sas-key=";
    /// let mut client = IotHubClient::from_connection_string(TwinType::Module, con_str, event_handler).unwrap();
    /// ```
    pub fn from_connection_string(
        twin_type: TwinType,
        connection_string: &str,
        event_handler: impl EventHandler + 'static,
    ) -> Result<Box<Self>, IotError> {
        IotHubClient::iothub_init()?;

        let mut twin: Box<dyn Twin> = match twin_type {
            TwinType::Module => Box::new(ModuleTwin::default()),
            TwinType::Device => Box::new(DeviceTwin::default()),
        };

        twin.create_from_connection_string(CString::new(connection_string)?)?;

        let mut client = Box::new(IotHubClient {
            twin,
            event_handler: Box::new(event_handler),
        });

        client.set_callbacks()?;

        Ok(client)
    }

    /// Let's you either create an outgoing D2C messages or parse an incoming cloud to device (C2D) messages.
    /// ```rust, no_run
    /// # use azure_iot_sdk::client::*;
    /// # struct MyEventHandler {}
    /// # impl EventHandler for MyEventHandler {}
    /// #
    /// #
    /// # let event_handler = MyEventHandler{};
    /// # let mut client = IotHubClient::from_identity_service(TwinType::Module, event_handler).unwrap();
    /// #
    /// let msg = IotMessage::builder()
    ///     .set_body(
    ///         serde_json::to_vec(r#"{"my telemetry message": "hi from device"}"#).unwrap(),
    ///     )
    ///     .set_id(String::from("my msg id"))
    ///     .set_correlation_id(String::from("my correleation id"))
    ///     .set_property(
    ///         String::from("my property key"),
    ///         String::from("my property value"),
    ///     )
    ///     .set_output_queue(String::from("my output queue"))
    ///     .build();
    ///
    /// client.send_d2c_message(msg);
    /// ```
    pub fn send_d2c_message(&mut self, mut message: IotMessage) -> Result<u32, IotError> {
        let handle = message.create_outgoing_handle()?;
        let queue = message.output_queue.clone();
        let ctx = rand::thread_rng().gen::<u32>();

        debug!("send_event with internal id: {}", ctx);

        self.twin.send_event_to_output_async(
            handle,
            queue,
            Some(IotHubClient::c_d2c_confirmation_callback),
            ctx,
        )?;

        Ok(ctx)
    }

    /// call this function in order to send a reported state update to iothub.
    /// ```rust, no_run
    /// # use azure_iot_sdk::client::*;
    /// # use serde_json::json;
    /// # struct MyEventHandler {}
    /// # impl EventHandler for MyEventHandler {}
    /// #
    /// #
    /// # let event_handler = MyEventHandler{};
    /// # let mut client = IotHubClient::from_identity_service(TwinType::Module, event_handler).unwrap();
    /// #
    /// let reported = json!({
    ///     "my_status": {
    ///         "status": "ok",
    ///         "timestamp": "2022-03-10",
    ///     }
    /// });
    ///
    /// client.send_reported_state(reported);
    /// ```
    pub fn send_reported_state(&mut self, reported: serde_json::Value) -> Result<(), IotError> {
        debug!("send reported: {}", reported.to_string());

        let reported_state = CString::new(reported.to_string())?;
        let size = reported_state.as_bytes().len();
        let ctx = self as *mut IotHubClient as *mut c_void;

        self.twin.send_reported_state(
            reported_state,
            size,
            Some(IotHubClient::c_reported_twin_callback),
            ctx,
        )
    }

    /// call this function periodically in order to trigger message computation,
    /// when work (sending/receiving) can be done by the client.
    /// [Microsoft recommends an interval of 100ms](https://docs.microsoft.com/en-us/azure/iot-hub/iot-c-sdk-ref/iothub-device-client-ll-h/iothubdeviceclient-ll-dowork)
    /// ```rust, no_run
    /// # use azure_iot_sdk::client::*;
    /// # use std::{thread, time};
    /// # struct MyEventHandler {}
    /// # impl EventHandler for MyEventHandler {}
    /// #
    /// # let event_handler = MyEventHandler{};
    /// # let mut client = IotHubClient::from_identity_service(TwinType::Module, event_handler).unwrap();
    /// #
    /// loop {
    ///     client.do_work();
    ///     thread::sleep(time::Duration::from_millis(100));
    /// }
    /// ```
    pub fn do_work(&mut self) {
        self.twin.do_work()
    }

    /// creates a [`DirectMethod`] from a closure
    /// ```no_run
    /// # use azure_iot_sdk::client::*;
    /// # use serde_json::json;
    /// #
    /// let mut dmm = DirectMethodMap::new();
    /// let dm = IotHubClient::make_direct_method(move |_in_json| Ok(None));
    ///
    /// dmm.insert(String::from("closure"), dm);
    /// ```
    pub fn make_direct_method<'a, F>(f: F) -> DirectMethod
    where
        F: Fn(serde_json::Value) -> Result<Option<serde_json::Value>, IotError> + 'static + Send,
    {
        Box::new(f) as DirectMethod
    }

    fn iothub_init() -> Result<(), IotError> {
        unsafe {
            IOTHUB_INIT_ONCE.call_once(|| {
                IOTHUB_INIT_RESULT = IoTHub_Init();
            });

            match IOTHUB_INIT_RESULT {
                0 => Ok(()),
                _ => Err(Box::<dyn Error + Send + Sync>::from(
                    "error while IoTHub_Init()",
                )),
            }
        }
    }

    fn set_callbacks(&mut self) -> Result<(), IotError> {
        let ctx = self as *mut IotHubClient as *mut c_void;

        self.twin.set_connection_status_callback(
            Some(IotHubClient::c_connection_status_callback),
            ctx,
        )?;
        self.twin
            .set_input_message_callback(Some(IotHubClient::c_c2d_message_callback), ctx)?;
        self.twin
            .set_twin_callback(Some(IotHubClient::c_desired_twin_callback), ctx)?;
        self.twin
            .get_twin_async(Some(IotHubClient::c_get_twin_async_callback), ctx)?;
        self.twin
            .set_method_callback(Some(IotHubClient::c_direct_method_callback), ctx)
    }

    unsafe extern "C" fn c_connection_status_callback(
        connection_status: IOTHUB_CLIENT_CONNECTION_STATUS,
        status_reason: IOTHUB_CLIENT_CONNECTION_STATUS_REASON,
        ctx: *mut ::std::os::raw::c_void,
    ) {
        let client = &mut *(ctx as *mut IotHubClient);
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

        debug!("Received connection status: {:?}", status);

        client.event_handler.handle_connection_status(status)
    }

    unsafe extern "C" fn c_c2d_message_callback(
        handle: *mut IOTHUB_MESSAGE_HANDLE_DATA_TAG,
        ctx: *mut ::std::os::raw::c_void,
    ) -> IOTHUBMESSAGE_DISPOSITION_RESULT {
        let client = &mut *(ctx as *mut IotHubClient);
        let mut property_keys: Vec<CString> = vec![];

        for p_str in client
            .event_handler
            .get_c2d_message_property_keys()
            .into_iter()
        {
            match CString::new(p_str) {
                Ok(p_cstr) => property_keys.push(p_cstr),
                Err(e) => {
                    error!(
                        "invalid property in c2d message received. payload: {}, error: {}",
                        p_str, e
                    );
                    return IOTHUBMESSAGE_DISPOSITION_RESULT_TAG_IOTHUBMESSAGE_REJECTED;
                }
            }
        }

        let message = IotMessage::from_incoming_handle(handle, property_keys);

        debug!("Received message from iothub: {:?}", message);

        match client.event_handler.handle_c2d_message(message) {
            Ok(_) => IOTHUBMESSAGE_DISPOSITION_RESULT_TAG_IOTHUBMESSAGE_ACCEPTED,
            Err(_) => IOTHUBMESSAGE_DISPOSITION_RESULT_TAG_IOTHUBMESSAGE_REJECTED,
        }
    }

    unsafe extern "C" fn c_desired_twin_callback(
        state: DEVICE_TWIN_UPDATE_STATE,
        payload: *const ::std::os::raw::c_uchar,
        size: usize,
        ctx: *mut ::std::os::raw::c_void,
    ) {
        match String::from_utf8(slice::from_raw_parts(payload, size).to_vec()) {
            Ok(p_str) => {
                match serde_json::from_str(&p_str) {
                    Ok(p_json) => {
                        let client = &mut *(ctx as *mut IotHubClient);
                        let state: TwinUpdateState = mem::transmute(state as i8);

                        debug!(
                            "Twin callback. state: {:?} size: {} payload: {}",
                            state, size, p_json
                        );

                        if let Err(e) = client.event_handler.handle_twin_desired(state, p_json) {
                            error!("client cannot handle desired twin: {}", e)
                        }
                    }
                    Err(e) => error!(
                        "desired twin cannot be parsed. payload: {} error: {}",
                        p_str, e
                    ),
                };
            }
            Err(e) => error!("desired twin cannot be parsed: {}", e),
        }
    }

    unsafe extern "C" fn c_get_twin_async_callback(
        state: DEVICE_TWIN_UPDATE_STATE,
        payload: *const ::std::os::raw::c_uchar,
        size: usize,
        _ctx: *mut ::std::os::raw::c_void,
    ) {
        let state: TwinUpdateState = mem::transmute(state as i8);
        match str::from_utf8(slice::from_raw_parts(payload, size)) {
            Ok(p) => debug!(
                "GetTwinAsync result. state: {:?} size: {} payload: {}",
                state, size, p
            ),
            Err(e) => error!("get twin async cannot parse payload: {}", e),
        }
    }

    unsafe extern "C" fn c_reported_twin_callback(
        status_code: std::os::raw::c_int,
        _ctx: *mut ::std::os::raw::c_void,
    ) {
        match status_code {
            204 => debug!("SendReportedTwin result: {}", status_code),
            _ => error!("SendReportedTwin result: {}", status_code),
        }
    }

    unsafe extern "C" fn c_direct_method_callback(
        method_name: *const ::std::os::raw::c_char,
        payload: *const ::std::os::raw::c_uchar,
        size: usize,
        response: *mut *mut ::std::os::raw::c_uchar,
        response_size: *mut usize,
        ctx: *mut ::std::os::raw::c_void,
    ) -> ::std::os::raw::c_int {
        const METHOD_RESPONSE_SUCCESS: i32 = 200;
        const METHOD_RESPONSE_ERROR: i32 = 401;

        let error;
        let empty_result: CString = CString::from_vec_unchecked(b"{ }".to_vec());
        *response_size = empty_result.as_bytes().len();
        *response = empty_result.into_raw() as *mut u8;

        let method_name = match CStr::from_ptr(method_name).to_str() {
            Ok(name) => name,
            Err(e) => {
                error!("cannot parse method name: {}", e);
                return METHOD_RESPONSE_ERROR;
            }
        };

        debug!("Received direct method call: {:?}", method_name);

        let client = &mut *(ctx as *mut IotHubClient);

        if let Some(method) = client
            .event_handler
            .get_direct_methods()
            .and_then(|methods| (methods.get(method_name)))
        {
            let payload: serde_json::Value =
                match str::from_utf8(slice::from_raw_parts(payload, size)) {
                    Ok(p) => match serde_json::from_str(p) {
                        Ok(json) => json,
                        Err(e) => {
                            error!("cannot parse direct method payload: {}", e);
                            return METHOD_RESPONSE_ERROR;
                        }
                    },
                    Err(e) => {
                        error!("cannot parse direct method payload: {}", e);
                        return METHOD_RESPONSE_ERROR;
                    }
                };

            debug!("Payload: {}", payload.to_string());

            match method(payload) {
                Result::Ok(None) => {
                    debug!("Method has no result");
                    return METHOD_RESPONSE_SUCCESS;
                }
                Result::Ok(Some(result)) => {
                    debug!("Result: {}", result.to_string());

                    match CString::new(result.to_string()) {
                        Ok(r) => {
                            *response_size = r.as_bytes().len();
                            *response = r.into_raw() as *mut u8;
                            return METHOD_RESPONSE_SUCCESS;
                        }
                        Err(e) => {
                            error!("cannot parse direct method result: {}", e);
                            return METHOD_RESPONSE_ERROR;
                        }
                    }
                }

                Result::Err(e) => error = e.to_string(),
            }
        } else {
            error = format!("method not implemented: {}", method_name)
        }

        error!("error: {}", error);

        match CString::new(json!(error).to_string()) {
            Ok(r) => {
                *response_size = r.as_bytes().len();
                *response = r.into_raw() as *mut u8;
            }
            Err(e) => {
                error!("cannot parse direct method result: {}", e);
            }
        }

        METHOD_RESPONSE_ERROR
    }

    unsafe extern "C" fn c_d2c_confirmation_callback(
        status: IOTHUB_CLIENT_RESULT,
        ctx: *mut std::ffi::c_void,
    ) {
        debug!(
            "Received confirmation from iothub for event with internal id: {} and status: {}",
            ctx as u32, status
        );
    }
}

impl Drop for IotHubClient {
    fn drop(&mut self) {
        self.twin.destroy()
    }
}
