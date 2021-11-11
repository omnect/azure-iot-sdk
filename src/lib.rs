use std::panic;
use std::ffi::{CString, c_void};
use std::{thread, time};
use std::convert::{TryFrom};
use std::str;
use std::sync::{Once};
use std::boxed::Box;
use std::ops::FnMut;
use serde_json::{ Value};
use libics_dm_azure_sys::*;

static IOTHUB: Once = Once::new();

/// Enum to describe the type of module event.
#[derive(Debug)]
pub enum IotHubModuleEvent {
    Twin(Value)
}
#[derive(Debug)]
pub enum IotHubModuleState {
    TwinState(DEVICE_TWIN_UPDATE_STATE)
}

pub struct IotHubModuleClient<'c> {
    handle: IOTHUB_MODULE_CLIENT_LL_HANDLE,
    callback: Box<dyn FnMut(IotHubModuleEvent, IotHubModuleState) + 'c>
}

unsafe impl<'c> Send for IotHubModuleClient<'c> {}

impl<'c> IotHubModuleClient<'c> {
    unsafe extern "C" fn c_unusedCallback(_status: i32, ctx: *mut std::ffi::c_void) {

    }

    unsafe extern "C" fn c_twin_callback(state: DEVICE_TWIN_UPDATE_STATE, payload: *const u8, size: u64, ctx: *mut std::ffi::c_void) {
        println!("Received twin callback from hub! {} {}", state, size);
        let client = &mut *(ctx as *mut IotHubModuleClient);
        let data = std::slice::from_raw_parts(payload, usize::try_from(size).unwrap());
        client.twin_callback(data, state);
    }

    fn twin_callback(&mut self, data: &[u8], state: DEVICE_TWIN_UPDATE_STATE) {
        let value = str::from_utf8(data).unwrap();
        let settings: Value = serde_json::from_slice(data).unwrap();
        println!("Received settings {} {}", settings, value);
        (self.callback)(IotHubModuleEvent::Twin(settings), IotHubModuleState::TwinState(state));
    }

    pub fn twin_report(&mut self, message: &String) {
        //create null pointer
        let mut null = 0;
        let null_ptr = &mut null as *mut _ as *mut std::ffi::c_void;
        unsafe {
            IoTHubModuleClient_LL_SendReportedState(self.handle , message.as_ptr(), message.len() as u64, Some(IotHubModuleClient::c_unusedCallback), null_ptr);
        }

    }

    pub fn new(callback: impl FnMut(IotHubModuleEvent, IotHubModuleState) + 'c) -> Box<Self> {
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
            let manual_connection_string = CString::new("HostName=iothub-ics-dev.azure-devices.net;DeviceId=joz_iot_connection_test-02:42:ac:14:00:03;ModuleId=iot-module-template;SharedAccessKey=HSrCe++YbM1aUq5mWes3dK111AVs0DA9baqxBSFLbvk=").unwrap();

            IOTHUB.call_once(|| { IoTHub_Init(); });

            let eis_provision_result: EISUtilityResult;
            let expiry_secs_since_epoch: u64;
            let information = &mut info as *mut _ as *mut ADUC_ConnectionInfo;

            match time::SystemTime::now().duration_since(time::SystemTime::UNIX_EPOCH) {
                Ok(n) =>
                {
                    expiry_secs_since_epoch = n.as_secs() + 7776000;
                },
                Err(_) => panic!("SystemTime before UNIX EPOCH!"),
            }
            eis_provision_result =  RequestConnectionStringFromEISWithExpiry(expiry_secs_since_epoch as i64, 2000 as u32, information);

            let handle;
            if (tagEISErr_EISErr_Ok != eis_provision_result.err) && (tagEISService_EISService_Utils != eis_provision_result.service) {
                println!("Error RequestConnectionStringFromEISWithExpiry: {} {}", eis_provision_result.err, eis_provision_result.service);
                handle = IoTHubModuleClient_LL_CreateFromConnectionString(manual_connection_string.as_ptr() , Some(MQTT_Protocol));
            }
            else {
                println!("OK RequestConnectionStringFromEISWithExpiry: {} {}", eis_provision_result.err, eis_provision_result.service);
                handle = IoTHubModuleClient_LL_CreateFromConnectionString(info.connectionString , Some(MQTT_Protocol));
            }

            if handle.is_null() {
                panic!("Failed to initialize the client from environment!");
            }

            let mut client = Box::new(IotHubModuleClient{ handle, callback: Box::new(callback) });
            let context = client.as_mut() as *mut IotHubModuleClient  as *mut c_void;

            if IoTHubModuleClient_LL_SetModuleTwinCallback(handle, Some(IotHubModuleClient::c_twin_callback), context) != IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK {
                panic!("Failed to set twin callback!");
            }
            return client;
        }
    }

    pub fn do_work(&mut self) {
        loop {
            unsafe { IoTHubModuleClient_LL_DoWork(self.handle); }
            let hundred_millis = time::Duration::from_millis(100);
            thread::sleep(hundred_millis);
        }
    }
}

impl<'c> Drop for IotHubModuleClient<'c> {
    fn drop(&mut self){
        unsafe {
            IoTHubModuleClient_LL_Destroy(self.handle);
        }
    }
}
