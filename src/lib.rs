use libics_dm_azure_sys::*;
use std::convert::TryFrom;
use std::str;
use std::time;

//The timeout for the Edge Identity Service HTTP requests
const EIS_PROVISIONING_TIMEOUT: u32 = 2000;
const SECONDS_IN_MONTH: u64 = 30 /* day/mo */ * 24 /* hr/day */ * 60 /* min/hr */ * 60 /*sec/min */;
//Time after startup the connection string will be provisioned for by the Edge Identity Service
const EIS_TOKEN_EXPIRY_TIME: u64 = 3 * SECONDS_IN_MONTH;

pub fn iot_hub_init() -> i32 {
    unsafe {
        let result = IoTHub_Init();
        result
    }
}

pub fn get_connection_info_from_identity_service() -> Result<*mut ::std::os::raw::c_char, String> {
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
    unsafe {
        let eis_provision_result: EISUtilityResult;
        let expiry_secs_since_epoch: u64;
        let information = &mut info as *mut _ as *mut ADUC_ConnectionInfo;

        match time::SystemTime::now().duration_since(time::SystemTime::UNIX_EPOCH) {
            Ok(n) => {
                expiry_secs_since_epoch = n.as_secs() + EIS_TOKEN_EXPIRY_TIME;
            }
            Err(_) => return Err("SystemTime before UNIX EPOCH!".to_string()),
        }
        eis_provision_result = RequestConnectionStringFromEISWithExpiry(
            expiry_secs_since_epoch as i64,
            EIS_PROVISIONING_TIMEOUT,
            information,
        );
        if (tagEISErr_EISErr_Ok != eis_provision_result.err)
            && (tagEISService_EISService_Utils != eis_provision_result.service)
        {
            return Err("RequestConnectionStringFromEISWithExpiry".to_string());
        } else {
            return Ok((*information).connectionString);
        }
    }
}

pub fn create_from_connection_string(
    connection_string: *mut ::std::os::raw::c_char,
) -> IOTHUB_MODULE_CLIENT_LL_HANDLE {
    unsafe {
        let handle = IoTHubModuleClient_LL_CreateFromConnectionString(
            connection_string,
            Some(MQTT_Protocol),
        );
        handle
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

unsafe extern "C" fn c_twin_callback(
    state: DEVICE_TWIN_UPDATE_STATE,
    payload: *const u8,
    size: u64,
    ctx: *mut std::ffi::c_void,
) {
    let data = std::slice::from_raw_parts(payload, usize::try_from(size).unwrap());
    let value = str::from_utf8(data).unwrap();

    println!(
        "Received twin callback from hub! {} {} {}",
        value, state, size
    );

    if state == DEVICE_TWIN_UPDATE_STATE_TAG_DEVICE_TWIN_UPDATE_PARTIAL {
        let _handle = ctx as *mut libics_dm_azure_sys::IOTHUB_MODULE_CLIENT_LL_HANDLE_DATA_TAG;
        let res = serde_json::from_str(value);
        if res.is_ok() {
            let mut twin_message: serde_json::Map<String, serde_json::Value> = res.unwrap();
            twin_message.remove_entry("$version");
            let message_serialised = serde_json::to_string(&twin_message).unwrap();

            //create null pointer
            let mut null = 0;
            let null_ptr = &mut null as *mut _ as *mut std::ffi::c_void;
            IoTHubModuleClient_LL_SendReportedState(
                _handle,
                message_serialised.as_ptr(),
                message_serialised.len() as u64,
                Some(c_unused_callback),
                null_ptr,
            );
        }
    }
}

unsafe extern "C" fn c_unused_callback(_status: i32, _ctx: *mut std::ffi::c_void) {}
