use crate::IotError;
use azure_iot_sdk_sys::*;
use std::boxed::Box;
use std::error::Error;
use std::ffi::{c_void, CString};

pub struct ModuleTwin {
    handle: IOTHUB_MODULE_CLIENT_LL_HANDLE,
}
pub struct DeviceTwin {
    handle: IOTHUB_DEVICE_CLIENT_LL_HANDLE,
}

pub enum TwinType {
    Device,
    Module,
}

pub trait Twin {
    fn new() -> Self;

    fn get_twin_type() -> TwinType;

    fn create_from_connection_string(&mut self, connection_string: CString)
        -> Result<(), IotError>;

    fn do_work(&mut self);

    fn destroy(&mut self);

    fn send_event_to_output_async(
        &mut self,
        message_handle: IOTHUB_MESSAGE_HANDLE,
        queue: CString,
        callback: IOTHUB_CLIENT_EVENT_CONFIRMATION_CALLBACK,
        ctx: u32,
    ) -> Result<(), IotError>;

    fn send_reported_state(
        &self,
        reported_state: CString,
        size: usize,
        callback: IOTHUB_CLIENT_REPORTED_STATE_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError>;

    fn set_connection_status_callback(
        &self,
        callback: IOTHUB_CLIENT_CONNECTION_STATUS_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError>;

    fn set_input_message_callback(
        &self,
        callback: IOTHUB_CLIENT_MESSAGE_CALLBACK_ASYNC,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError>;

    fn set_twin_callback(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_TWIN_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError>;

    fn get_twin_async(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_TWIN_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError>;

    fn set_method_callback(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_METHOD_CALLBACK_ASYNC,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError>;
}

impl Twin for ModuleTwin {
    fn new() -> Self {
        ModuleTwin {
            handle: std::ptr::null_mut(),
        }
    }

    fn get_twin_type() -> TwinType {
        TwinType::Module
    }

    fn create_from_connection_string(
        &mut self,
        connection_string: CString,
    ) -> Result<(), IotError> {
        unsafe {
            self.handle = IoTHubModuleClient_LL_CreateFromConnectionString(
                connection_string.into_raw(),
                Some(MQTT_Protocol),
            );

            if self.handle.is_null() {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_CreateFromConnectionString()",
                ));
            }

            Ok(())
        }
    }

    fn do_work(&mut self) {
        unsafe {
            IoTHubModuleClient_LL_DoWork(self.handle);
        }
    }

    fn destroy(&mut self) {
        unsafe {
            IoTHubModuleClient_LL_Destroy(self.handle);
        }
    }

    fn send_event_to_output_async(
        &mut self,
        message_handle: IOTHUB_MESSAGE_HANDLE,
        queue: CString,
        callback: IOTHUB_CLIENT_EVENT_CONFIRMATION_CALLBACK,
        ctx: u32,
    ) -> Result<(), IotError> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_LL_SendEventToOutputAsync(
                    self.handle,
                    message_handle,
                    queue.as_ptr(),
                    callback,
                    ctx as *mut c_void,
                )
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_SendEventToOutputAsync()",
                ));
            }

            Ok(())
        }
    }

    fn send_reported_state(
        &self,
        reported_state: CString,
        size: usize,
        callback: IOTHUB_CLIENT_REPORTED_STATE_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_LL_SendReportedState(
                    self.handle,
                    reported_state.into_raw() as *mut u8,
                    size,
                    callback,
                    ctx,
                )
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_SendReportedState()",
                ));
            }

            Ok(())
        }
    }

    fn set_connection_status_callback(
        &self,
        callback: IOTHUB_CLIENT_CONNECTION_STATUS_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_LL_SetConnectionStatusCallback(self.handle, callback, ctx)
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_SetConnectionStatusCallback()",
                ));
            }

            Ok(())
        }
    }

    fn set_input_message_callback(
        &self,
        callback: IOTHUB_CLIENT_MESSAGE_CALLBACK_ASYNC,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError> {
        unsafe {
            let input_name = CString::new("input").unwrap();
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_LL_SetInputMessageCallback(
                    self.handle,
                    input_name.as_ptr(),
                    callback,
                    ctx,
                )
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_SetInputMessageCallback()",
                ));
            }

            Ok(())
        }
    }

    fn set_twin_callback(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_TWIN_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_LL_SetModuleTwinCallback(self.handle, callback, ctx)
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_SetModuleTwinCallback()",
                ));
            }

            Ok(())
        }
    }

    fn get_twin_async(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_TWIN_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_LL_GetTwinAsync(self.handle, callback, ctx)
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_GetTwinAsync()",
                ));
            }

            Ok(())
        }
    }

    fn set_method_callback(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_METHOD_CALLBACK_ASYNC,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_LL_SetModuleMethodCallback(self.handle, callback, ctx)
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_SetModuleMethodCallback()",
                ));
            }

            Ok(())
        }
    }
}

impl Twin for DeviceTwin {
    fn new() -> Self {
        DeviceTwin {
            handle: std::ptr::null_mut(),
        }
    }

    fn get_twin_type() -> TwinType {
        TwinType::Device
    }

    fn create_from_connection_string(
        &mut self,
        connection_string: CString,
    ) -> Result<(), IotError> {
        unsafe {
            self.handle = IoTHubDeviceClient_LL_CreateFromConnectionString(
                connection_string.into_raw(),
                Some(MQTT_Protocol),
            );

            if self.handle.is_null() {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubDeviceClient_LL_CreateFromConnectionString()",
                ));
            }
            Ok(())
        }
    }

    fn do_work(&mut self) {
        unsafe {
            IoTHubDeviceClient_LL_DoWork(self.handle);
        }
    }

    fn destroy(&mut self) {
        unsafe {
            IoTHubDeviceClient_LL_Destroy(self.handle);
        }
    }

    fn send_event_to_output_async(
        &mut self,
        message_handle: IOTHUB_MESSAGE_HANDLE,
        _queue: CString,
        callback: IOTHUB_CLIENT_EVENT_CONFIRMATION_CALLBACK,
        ctx: u32,
    ) -> Result<(), IotError> {
        unsafe {
            let result = IoTHubDeviceClient_LL_SendEventAsync(
                self.handle,
                message_handle,
                callback,
                ctx as *mut c_void,
            );

            if result != IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubDeviceClient_LL_SendEventAsync()",
                ));
            }

            Ok(())
        }
    }

    fn send_reported_state(
        &self,
        reported_state: CString,
        size: usize,
        callback: IOTHUB_CLIENT_REPORTED_STATE_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubDeviceClient_LL_SendReportedState(
                    self.handle,
                    reported_state.into_raw() as *mut u8,
                    size,
                    callback,
                    ctx,
                )
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubDeviceClient_LL_SendReportedState()",
                ));
            }

            Ok(())
        }
    }

    fn set_connection_status_callback(
        &self,
        callback: IOTHUB_CLIENT_CONNECTION_STATUS_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubDeviceClient_LL_SetConnectionStatusCallback(self.handle, callback, ctx)
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubDeviceClient_LL_SetConnectionStatusCallback()",
                ));
            }

            Ok(())
        }
    }

    fn set_input_message_callback(
        &self,
        callback: IOTHUB_CLIENT_MESSAGE_CALLBACK_ASYNC,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubDeviceClient_LL_SetMessageCallback(self.handle, callback, ctx)
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubDeviceClient_LL_SetMessageCallback()",
                ));
            }

            Ok(())
        }
    }

    fn set_twin_callback(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_TWIN_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubDeviceClient_LL_SetDeviceTwinCallback(self.handle, callback, ctx)
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubDeviceClient_LL_SetDeviceTwinCallback()",
                ));
            }

            Ok(())
        }
    }

    fn get_twin_async(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_TWIN_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubDeviceClient_LL_GetTwinAsync(self.handle, callback, ctx)
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubDeviceClient_LL_GetTwinAsync()",
                ));
            }

            Ok(())
        }
    }

    fn set_method_callback(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_METHOD_CALLBACK_ASYNC,
        ctx: *mut std::ffi::c_void,
    ) -> Result<(), IotError> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubDeviceClient_LL_SetDeviceMethodCallback(self.handle, callback, ctx)
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubDeviceClient_LL_SetDeviceMethodCallback()",
                ));
            }

            Ok(())
        }
    }
}
