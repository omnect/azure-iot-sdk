use crate::message::IotMessage;
use crate::twin::{Twin, TwinType};
use crate::IotError;
use azure_iot_sdk_sys::*;
use core::slice;
use eis_utils::*;
use log::{debug, error};
use rand::Rng;
use std::boxed::Box;
use std::collections::HashMap;
use std::error::Error;
use std::ffi::{c_void, CStr, CString};
use std::mem;
use std::str;
use std::sync::Once;
use std::time::{Duration, SystemTime};

static mut IOTHUB_INIT_RESULT: i32 = -1;
static IOTHUB_INIT_ONCE: Once = Once::new();

macro_rules! days_to_secs {
    ($num_days:expr) => {
        $num_days * 24 * 60 * 60
    };
}

#[derive(Debug)]
pub enum TwinUpdateState {
    Complete = 0,
    Partial = 1,
}

#[derive(Debug)]
pub enum UnauthenticatedReason {
    ExpiredSasToken,
    DeviceDisabled,
    BadCredential,
    RetryExpired,
    NoNetwork,
    CommunicationError,
}

#[derive(Debug)]
pub enum AuthenticationStatus {
    Authenticated,
    Unauthenticated(UnauthenticatedReason),
}

pub type DirectMethod =
    Box<(dyn Fn(serde_json::Value) -> Result<Option<serde_json::Value>, IotError> + Send)>;

pub trait EventHandler {
    fn handle_connection_status(&self, auth_status: AuthenticationStatus) {
        debug!(
            "unhandled call to handle_connection_status(). status: {:?}",
            auth_status
        )
    }

    fn handle_c2d_message(&self, message: IotMessage) -> Result<(), IotError> {
        debug!("unhandled call to handle_message(). message: {:?}", message);
        Ok(())
    }

    fn get_c2d_message_property_keys(&self) -> Vec<&'static str> {
        vec![]
    }

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

    fn get_direct_methods(&self) -> Option<&HashMap<String, DirectMethod>> {
        debug!("unhandled call to get_direct_methods().");
        None
    }
}

pub struct IotHubClient<T: Twin> {
    twin: T,
    event_handler: Box<dyn EventHandler>,
}

impl<T: Twin> IotHubClient<T>
where
    T: Twin,
{
    pub fn from_identity_service(
        event_handler: impl EventHandler + 'static,
    ) -> Result<Box<Self>, IotError> {
        let connection_info = request_connection_string_from_eis_with_expiry(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)?
                .saturating_add(Duration::from_secs(days_to_secs!(30))),
        )
        .map_err(|err| {
            if let TwinType::Device = T::get_twin_type() {
                error!("iot identity service failed to create device twin identity.
                In case you use TPM attestation please note that this combination is currently not supported.");
            }

            err
        })?;

        IotHubClient::from_connection_string(
            connection_info.connection_string.as_str(),
            event_handler,
        )
    }

    pub fn from_connection_string(
        connection_string: &str,
        event_handler: impl EventHandler + 'static,
    ) -> Result<Box<Self>, IotError> {
        IotHubClient::<T>::iothub_init()?;

        let mut twin: T = T::new();

        twin.create_from_connection_string(CString::new(connection_string)?)?;

        let mut client = Box::new(IotHubClient::<T> {
            twin,
            event_handler: Box::new(event_handler),
        });

        client.set_callbacks()?;

        Ok(client)
    }

    pub fn send_d2c_message(&mut self, mut message: IotMessage) -> Result<u32, IotError> {
        let handle = message.create_outgoing_handle()?;
        let queue = message.output_queue.clone();
        let ctx = rand::thread_rng().gen::<u32>();

        debug!("send_event with internal id: {}", ctx);

        self.twin.send_event_to_output_async(
            handle,
            queue,
            Some(IotHubClient::<T>::c_d2c_confirmation_callback),
            ctx,
        )?;

        Ok(ctx)
    }

    pub fn send_reported_state(&mut self, reported: serde_json::Value) -> Result<(), IotError> {
        debug!("send reported: {}", reported.to_string());

        let reported_state = CString::new(reported.to_string()).unwrap();
        let size = reported_state.as_bytes().len();
        let ctx = self as *mut IotHubClient<T> as *mut c_void;

        self.twin.send_reported_state(
            reported_state,
            size,
            Some(IotHubClient::<T>::c_reported_twin_callback),
            ctx,
        )
    }

    pub fn do_work(&mut self) {
        self.twin.do_work()
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
        let ctx = self as *mut IotHubClient<T> as *mut c_void;

        self.twin.set_connection_status_callback(
            Some(IotHubClient::<T>::c_connection_status_callback),
            ctx,
        )?;
        self.twin
            .set_input_message_callback(Some(IotHubClient::<T>::c_c2d_message_callback), ctx)?;
        self.twin
            .set_twin_callback(Some(IotHubClient::<T>::c_desired_twin_callback), ctx)?;
        self.twin
            .get_twin_async(Some(IotHubClient::<T>::c_get_twin_async_callback), ctx)?;
        self.twin
            .set_method_callback(Some(IotHubClient::<T>::c_direct_method_callback), ctx)
    }

    unsafe extern "C" fn c_connection_status_callback(
        connection_status: IOTHUB_CLIENT_CONNECTION_STATUS,
        status_reason: IOTHUB_CLIENT_CONNECTION_STATUS_REASON,
        ctx: *mut ::std::os::raw::c_void,
    ) {
        let client = &mut *(ctx as *mut IotHubClient<T>);
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
        let client = &mut *(ctx as *mut IotHubClient<T>);

        let property_keys = client
            .event_handler
            .get_c2d_message_property_keys()
            .into_iter()
            .map(|s| CString::new(s).unwrap())
            .collect();

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
        let payload: serde_json::Value =
            serde_json::from_slice(slice::from_raw_parts(payload, size)).unwrap();
        let client = &mut *(ctx as *mut IotHubClient<T>);
        let state: TwinUpdateState = mem::transmute(state as i8);

        debug!(
            "Twin callback. state: {:?} size: {} payload: {}",
            state, size, payload
        );

        client
            .event_handler
            .handle_twin_desired(state, payload)
            .unwrap();
    }

    unsafe extern "C" fn c_get_twin_async_callback(
        state: DEVICE_TWIN_UPDATE_STATE,
        payload: *const ::std::os::raw::c_uchar,
        size: usize,
        _ctx: *mut ::std::os::raw::c_void,
    ) {
        let state: TwinUpdateState = mem::transmute(state as i8);

        debug!(
            "GetTwinAsync result. state: {:?} size: {} payload: {}",
            state,
            size,
            str::from_utf8(slice::from_raw_parts(payload, size)).unwrap()
        );
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

        let empty_result: CString = CString::new("{ }").unwrap();
        *response_size = empty_result.as_bytes().len();
        *response = empty_result.into_raw() as *mut u8;

        let method_name = CStr::from_ptr(method_name).to_str().unwrap();
        debug!("Received direct method call: {:?}", method_name);

        let client = &mut *(ctx as *mut IotHubClient<T>);

        if let Some(method) = client
            .event_handler
            .get_direct_methods()
            .and_then(|methods| (methods.get(method_name)))
        {
            let payload = slice::from_raw_parts(payload, size);
            let payload: serde_json::Value =
                serde_json::from_str(str::from_utf8(payload).unwrap()).unwrap();
            debug!("Payload: {}", payload.to_string());

            match method(payload) {
                Result::Ok(None) => {
                    debug!("Method has no result");
                    return METHOD_RESPONSE_SUCCESS;
                }
                Result::Ok(Some(result)) => {
                    debug!("Result: {}", result.to_string());

                    let result: CString = CString::new(result.to_string()).unwrap();
                    *response_size = result.as_bytes().len();
                    *response = result.into_raw() as *mut u8;

                    return METHOD_RESPONSE_SUCCESS;
                }

                Result::Err(e) => error!("error: {}", e),
            }
        } else {
            error!("method not implemented: {}", method_name)
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

impl<T: Twin> Drop for IotHubClient<T> {
    fn drop(&mut self) {
        self.twin.destroy()
    }
}
