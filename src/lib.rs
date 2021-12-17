use ics_dm_azure_sys::*;
use log::{debug, info};
use std::convert::TryFrom;
use std::convert::TryInto;
use std::ffi::CString;
use std::str;
use std::sync::Once;
use std::time;

//The timeout for the Edge Identity Service HTTP requests
const EIS_PROVISIONING_TIMEOUT: u32 = 2000;

const SECONDS_IN_MONTH: u64 = 30 /* day/mo */ * 24 /* hr/day */ * 60 /* min/hr */ * 60 /*sec/min */;
//Time after startup the connection string will be provisioned for by the Edge Identity Service
//NOTICE: Such a long expiry date of 3 months was set for demo purposes only.
const EIS_TOKEN_EXPIRY_TIME: u64 = 3 * SECONDS_IN_MONTH;

static IOTHUB: Once = Once::new();

pub fn iot_hub_init() -> Result<(), String> {
    unsafe {
        let mut result: i32 = 1;
        IOTHUB.call_once(|| {
            result = IoTHub_Init();
        });
        if 0 == result {
            return Ok(());
        } else {
            return Err("iot_hub_init not OK!".to_string());
        }
    }
}

pub fn get_connection_info_from_identity_service() -> Result<String, String> {
    //create null pointer
    let mut null = 0 as u8;
    let null_char_ptr = &mut null as *mut _ as *mut std::os::raw::c_char;

    let connection_string: *mut ::std::os::raw::c_char = null_char_ptr;
    let certificate_string: *mut ::std::os::raw::c_char = null_char_ptr;
    let openssl_engine: *mut ::std::os::raw::c_char = null_char_ptr;
    let openssl_private_key: *mut ::std::os::raw::c_char = null_char_ptr;
    let mut info = ADUC_ConnectionInfo {
        authType: tagADUC_AuthType_ADUC_AuthType_NotSet,
        connType: tagADUC_ConnType_ADUC_ConnType_NotSet,
        connectionString: connection_string,
        certificateString: certificate_string,
        opensslEngine: openssl_engine,
        opensslPrivateKey: openssl_private_key,
    };
    let eis_provision_result: EISUtilityResult;
    let expiry_secs_since_epoch: u64;
    let information = &mut info as *mut _ as *mut ADUC_ConnectionInfo;

    match time::SystemTime::now().duration_since(time::SystemTime::UNIX_EPOCH) {
        Ok(n) => {
            expiry_secs_since_epoch = n.as_secs() + EIS_TOKEN_EXPIRY_TIME;
        }
        Err(_) => return Err("SystemTime before UNIX EPOCH!".to_string()),
    }
    unsafe {
        eis_provision_result = RequestConnectionStringFromEISWithExpiry(
            expiry_secs_since_epoch.try_into().unwrap(),
            EIS_PROVISIONING_TIMEOUT,
            information,
        );
        if (tagEISErr_EISErr_Ok != eis_provision_result.err)
            && (tagEISService_EISService_Utils != eis_provision_result.service)
        {
            return Err("RequestConnectionStringFromEISWithExpiry".to_string());
        } else {
            let c_string = CString::from_raw((*information).connectionString);
            return Ok(c_string.into_string().unwrap());
        }
    }
}

pub fn create_from_connection_string(
    connection_string: String,
) -> Result<IOTHUB_MODULE_CLIENT_LL_HANDLE, String> {
    unsafe {
        let c_string =
            CString::new(connection_string).expect("CString::new connection_string failed");
        let handle = IoTHubModuleClient_LL_CreateFromConnectionString(
            c_string.into_raw(),
            Some(MQTT_Protocol),
        );

        // enable azure sdk logging
        // let c_logtrace = CString::new("logtrace").expect("CString::logtrace failed");
        // let mut logging = 1 as u8;
        // let logging_void_ptr = &mut logging as *mut _ as *mut std::os::raw::c_void;
        // IoTHubModuleClient_LL_SetOption(handle, c_logtrace.into_raw(), logging_void_ptr);

        let handle_c_void = handle as *mut std::ffi::c_void;
        IoTHubModuleClient_LL_SetConnectionStatusCallback(
            handle,
            Some(c_connection_callback),
            handle_c_void,
        );

        if handle.is_null() {
            return Err("no valid handle received".to_string());
        } else {
            return Ok(handle);
        }
    }
}

pub fn set_module_twin_callback(handle: IOTHUB_MODULE_CLIENT_LL_HANDLE) -> Result<(), String> {
    let handle_c_void = handle as *mut std::ffi::c_void;
    unsafe {
        if IoTHubModuleClient_LL_SetModuleTwinCallback(handle, Some(c_twin_callback), handle_c_void)
            != IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
        {
            return Err("Failed to set twin callback!".to_string());
        }
        Ok(())
    }
}

pub fn do_work(handle: IOTHUB_MODULE_CLIENT_LL_HANDLE) {
    unsafe {
        IoTHubModuleClient_LL_DoWork(handle);
    }
}

unsafe extern "C" fn c_connection_callback(
    connection_status: IOTHUB_CLIENT_CONNECTION_STATUS,
    status_reason: IOTHUB_CLIENT_CONNECTION_STATUS_REASON,
    _ctx: *mut ::std::os::raw::c_void,
) {
    info!(
        "c_connection_callback! {} {}",
        connection_status, status_reason
    );
}

unsafe extern "C" fn c_twin_callback(
    state: DEVICE_TWIN_UPDATE_STATE,
    payload: *const u8,
    size: usize,
    ctx: *mut std::ffi::c_void,
) {
    let data = std::slice::from_raw_parts(payload, usize::try_from(size).unwrap());
    let value = str::from_utf8(data).unwrap();

    debug!(
        "Received twin callback from hub! {} {} {}",
        value, state, size
    );

    match state {
        DEVICE_TWIN_UPDATE_STATE_TAG_DEVICE_TWIN_UPDATE_PARTIAL => {
            let _handle = ctx as *mut ics_dm_azure_sys::IOTHUB_MODULE_CLIENT_LL_HANDLE_DATA_TAG;

            let mut twin_message: serde_json::Map<String, serde_json::Value>;
            match serde_json::from_str(value) {
                Ok(js) => {
                    twin_message = js;
                    twin_message.remove_entry("$version");
                    let message_serialised = serde_json::to_string(&twin_message).unwrap();

                    //create null pointer
                    let mut null = 0;
                    let null_ptr = &mut null as *mut _ as *mut std::ffi::c_void;
                    IoTHubModuleClient_LL_SendReportedState(
                        _handle,
                        message_serialised.as_ptr(),
                        message_serialised.len(),
                        Some(c_send_reported_callback),
                        null_ptr,
                    );
                }
                Err(_) => debug!("Error Received twin callback parse json"),
            }
        }
        _ => debug!("only DEVICE_TWIN_UPDATE_PARTIAL is processed"),
    }
}

unsafe extern "C" fn c_send_reported_callback(_status: i32, _ctx: *mut std::ffi::c_void) {
    debug!("c_send_reported_callback status: {}", _status);
}
