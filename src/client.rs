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

use crate::message::IotMessage;

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

pub type DirectMethod = Box<
    (dyn Fn(serde_json::Value) -> Result<Option<serde_json::Value>, Box<dyn Error + Send + Sync>>
         + Send),
>;

pub trait EventHandler {
    fn handle_connection_status(
        &self,
        _auth_status: AuthenticationStatus,
    ) {
    }

    fn handle_c2d_message(&self, message: IotMessage) -> Result<(), Box<dyn Error + Send + Sync>> {
        debug!("unhandled call to handle_message(). message: {:?}", message);
        Ok(())
    }

    fn handle_twin_desired(
        &self,
        state: TwinUpdateState,
        desired: serde_json::Value,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
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

pub struct IotHubModuleClient {
    handle: IOTHUB_MODULE_CLIENT_LL_HANDLE,
    event_handler: Box<dyn EventHandler>,
}

impl IotHubModuleClient {
    pub fn from_identity_service(
        event_handler: impl EventHandler + 'static,
    ) -> Result<Box<Self>, Box<dyn Error + Send + Sync>> {
        let connection_info = request_connection_string_from_eis_with_expiry(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)?
                .saturating_add(Duration::from_secs(days_to_secs!(30))),
        )?;

        IotHubModuleClient::from_connection_string(
            connection_info.connection_string.as_str(),
            event_handler,
        )
    }

    pub fn from_connection_string(
        connection_string: &str,
        event_handler: impl EventHandler + 'static,
    ) -> Result<Box<Self>, Box<dyn Error + Send + Sync>> {
        IotHubModuleClient::iothub_init()?;

        unsafe {
            let c_string = CString::new(connection_string)?;
            let handle = IoTHubModuleClient_LL_CreateFromConnectionString(
                c_string.into_raw(),
                Some(MQTT_Protocol),
            );

            if handle.is_null() {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_CreateFromConnectionString()",
                ));
            }

            let mut client = Box::new(IotHubModuleClient {
                handle,
                event_handler: Box::new(event_handler),
            });

            client.set_callbacks()?;

            Ok(client)
        }
    }

    pub fn send_d2c_message(
        &mut self,
        mut message: IotMessage,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        unsafe {
            let handle = message.create_outgoing_handle()?;
            let queue = message.get_output_queue();
            let ctx = rand::thread_rng().gen::<u32>();

            debug!("send_event with internal id: {}", ctx);

            let result = IoTHubModuleClient_LL_SendEventToOutputAsync(
                self.handle,
                handle,
                queue.as_ptr(),
                Some(IotHubModuleClient::c_d2c_confirmation_callback),
                ctx as *mut c_void,
            );

            if result != IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_SendEventToOutputAsync()",
                ));
            }
        }

        Ok(())
    }

    pub fn send_reported_state(
        &mut self,
        reported: serde_json::Value,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        unsafe {
            debug!("send reported: {}", reported.to_string());

            let reported_state = CString::new(reported.to_string()).unwrap();
            let size = reported_state.as_bytes().len();
            let ctx = self as *mut IotHubModuleClient as *mut c_void;

            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_LL_SendReportedState(
                    self.handle,
                    reported_state.into_raw() as *mut u8,
                    size,
                    Some(IotHubModuleClient::c_reported_twin_callback),
                    ctx,
                )
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_SendReportedState()",
                ));
            }
        }

        Ok(())
    }

    pub fn do_work(&mut self) {
        unsafe {
            IoTHubModuleClient_LL_DoWork(self.handle);
        }
    }

    fn iothub_init() -> Result<(), Box<dyn Error + Send + Sync>> {
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

    fn set_callbacks(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        unsafe {
            let ctx = self as *mut IotHubModuleClient as *mut c_void;

            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_LL_SetConnectionStatusCallback(
                    self.handle,
                    Some(IotHubModuleClient::c_connection_status_callback),
                    ctx,
                )
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_SetConnectionStatusCallback()",
                ));
            }

            let input_name = CString::new("input").unwrap();
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_LL_SetInputMessageCallback(
                    self.handle,
                    input_name.as_ptr(),
                    Some(IotHubModuleClient::c_c2d_message_callback),
                    ctx,
                )
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_SetInputMessageCallback()",
                ));
            }

            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_LL_SetModuleTwinCallback(
                    self.handle,
                    Some(IotHubModuleClient::c_desired_twin_callback),
                    ctx,
                )
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_SetModuleTwinCallback()",
                ));
            }

            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_LL_GetTwinAsync(
                    self.handle,
                    Some(IotHubModuleClient::c_get_twin_async_callback),
                    ctx,
                )
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_GetTwinAsync()",
                ));
            }

            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_LL_SetModuleMethodCallback(
                    self.handle,
                    Some(IotHubModuleClient::c_direct_method_callback),
                    ctx,
                )
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_SetModuleMethodCallback()",
                ));
            }

            Ok(())
        }
    }

    //#[allow(unused_variables, non_snake_case)]
    unsafe extern "C" fn c_connection_status_callback(
        connection_status: IOTHUB_CLIENT_CONNECTION_STATUS,
        status_reason: IOTHUB_CLIENT_CONNECTION_STATUS_REASON,
        ctx: *mut ::std::os::raw::c_void,
    ) {
        let client = &mut *(ctx as *mut IotHubModuleClient);
        let status = match connection_status {
            IOTHUB_CLIENT_CONNECTION_STATUS_TAG_IOTHUB_CLIENT_CONNECTION_AUTHENTICATED => {
                AuthenticationStatus::Authenticated
            }
            IOTHUB_CLIENT_CONNECTION_STATUS_TAG_IOTHUB_CLIENT_CONNECTION_UNAUTHENTICATED => {
                match status_reason {
                    IOTHUB_CLIENT_CONNECTION_STATUS_REASON_TAG_IOTHUB_CLIENT_CONNECTION_EXPIRED_SAS_TOKEN => {
                        AuthenticationStatus::Unauthenticated(UnauthenticatedReason::ExpiredSasToken)
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
                        AuthenticationStatus::Unauthenticated(UnauthenticatedReason::CommunicationError)
                    }
                    _ => panic!("unknown unauthenticated reason"),
                }
            }
            _ => panic!("unknown authenticated state"),
        };

        debug!("Received connection status: {:?}", status);

        client
            .event_handler
            .handle_connection_status(status)
    }

    unsafe extern "C" fn c_c2d_message_callback(
        handle: *mut IOTHUB_MESSAGE_HANDLE_DATA_TAG,
        ctx: *mut ::std::os::raw::c_void,
    ) -> IOTHUBMESSAGE_DISPOSITION_RESULT {
        let message = IotMessage::from_incoming_handle(handle);
        let client = &mut *(ctx as *mut IotHubModuleClient);

        debug!("Received message from iothub: {:?}", message);

        match client.event_handler.handle_c2d_message(message) {
            Result::Ok(_) => IOTHUBMESSAGE_DISPOSITION_RESULT_TAG_IOTHUBMESSAGE_ACCEPTED,
            Result::Err(_) => IOTHUBMESSAGE_DISPOSITION_RESULT_TAG_IOTHUBMESSAGE_REJECTED,
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
        let client = &mut *(ctx as *mut IotHubModuleClient);
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

        let client = &mut *(ctx as *mut IotHubModuleClient);

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
            error!("method not implemented")
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

impl Drop for IotHubModuleClient {
    fn drop(&mut self) {
        unsafe {
            IoTHubModuleClient_LL_Destroy(self.handle);
        }
    }
}
