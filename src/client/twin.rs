use crate::client::IotError;
use azure_iot_sdk_sys::*;
use std::boxed::Box;
use std::error::Error;
use std::ffi::{c_void, CStr, CString};

#[cfg(any(feature = "module_client", feature = "edge_client"))]
#[derive(Default, Debug)]
pub struct ModuleTwin {
    handle: Option<IOTHUB_MODULE_CLIENT_LL_HANDLE>,
}

#[cfg(feature = "device_client")]
#[derive(Default, Debug)]
pub struct DeviceTwin {
    handle: Option<IOTHUB_DEVICE_CLIENT_LL_HANDLE>,
}

/// type of client twin
#[derive(Debug, PartialEq)]
pub enum ClientType {
    /// edge module twin client
    Edge,
    /// module twin client
    Module,
    /// device twin client
    Device,
}

pub(crate) fn get_sdk_version_string() -> String {
    unsafe {
        let version_string = IoTHubClient_GetVersionString();

        if version_string.is_null() {
            return String::from("unknown azure-sdk-c version string");
        }

        CStr::from_ptr(version_string)
            .to_str()
            .unwrap_or("invalid azure-sdk-c version string")
            .to_owned()
    }
}

pub trait Twin {
    #[cfg(any(feature = "module_client", feature = "device_client"))]
    fn create_from_connection_info(
        &mut self,
        connection_info: &eis_utils::ConnectionInfo,
    ) -> Result<(), IotError>;

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

#[cfg(feature = "edge_client")]
impl ModuleTwin {
    pub(crate) fn create_from_edge_environment(&mut self) -> Result<(), IotError> {
        unsafe {
            let handle = IoTHubModuleClient_LL_CreateFromEnvironment(Some(MQTT_Protocol));

            if handle.is_null() {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_CreateFromEnvironment()",
                ));
            }

            self.handle = Some(handle);

            Ok(())
        }
    }
}

#[cfg(any(feature = "module_client", feature = "edge_client"))]
impl Twin for ModuleTwin {
    #[cfg(any(feature = "module_client"))]
    fn create_from_connection_info(
        &mut self,
        connection_info: &eis_utils::ConnectionInfo,
    ) -> Result<(), IotError> {
        // todo Error when connection_info includes device identity info
        unsafe {
            let handle = IoTHubModuleClient_LL_CreateFromConnectionString(
                CString::new(connection_info.connection_string.clone())?.into_raw(),
                Some(MQTT_Protocol),
            );

            if handle.is_null() {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_CreateFromConnectionString()",
                ));
            }

            self.handle = Some(handle);

            // currently not supported by iot-identity-service for module twin, but it's planned:
            // x509

            Ok(())
        }
    }

    fn create_from_connection_string(
        &mut self,
        connection_string: CString,
    ) -> Result<(), IotError> {
        unsafe {
            let handle = IoTHubModuleClient_LL_CreateFromConnectionString(
                connection_string.into_raw(),
                Some(MQTT_Protocol),
            );

            if handle.is_null() {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubModuleClient_LL_CreateFromConnectionString()",
                ));
            }

            self.handle = Some(handle);

            Ok(())
        }
    }

    fn do_work(&mut self) {
        unsafe {
            IoTHubModuleClient_LL_DoWork(self.handle.unwrap());
        }
    }

    fn destroy(&mut self) {
        unsafe {
            IoTHubModuleClient_LL_Destroy(self.handle.unwrap());
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
                    self.handle.unwrap(),
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
                    self.handle.unwrap(),
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
                != IoTHubModuleClient_LL_SetConnectionStatusCallback(
                    self.handle.unwrap(),
                    callback,
                    ctx,
                )
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
            let input_name = CString::new("input")?;
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_LL_SetInputMessageCallback(
                    self.handle.unwrap(),
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
                != IoTHubModuleClient_LL_SetModuleTwinCallback(self.handle.unwrap(), callback, ctx)
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
                != IoTHubModuleClient_LL_GetTwinAsync(self.handle.unwrap(), callback, ctx)
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
                != IoTHubModuleClient_LL_SetModuleMethodCallback(
                    self.handle.unwrap(),
                    callback,
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
}

#[cfg(feature = "device_client")]
impl Twin for DeviceTwin {
    fn create_from_connection_info(
        &mut self,
        connection_info: &eis_utils::ConnectionInfo,
    ) -> Result<(), IotError> {
        // todo Error when connection_info includes device identity info
        unsafe {
            let handle = IoTHubDeviceClient_LL_CreateFromConnectionString(
                CString::new(connection_info.connection_string.clone())?.into_raw(),
                Some(MQTT_Protocol),
            );

            if handle.is_null() {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubDeviceClient_LL_CreateFromConnectionString()",
                ));
            }

            if eis_utils::AuthType::SASCert == connection_info.auth_type {
                // I was not able to use OPTION_X509_CERT and OPTION_X509_PRIVATE_KEY as parameters
                // We should revisit how to correctly cast these.
                if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                    != IoTHubDeviceClient_LL_SetOption(
                        handle,
                        CString::new("x509certificate")?.into_raw()
                            as *const ::std::os::raw::c_char,
                        CString::new(connection_info.certificate_string.clone())?.into_raw()
                            as *const c_void,
                    )
                    || IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                        != IoTHubDeviceClient_LL_SetOption(
                            handle,
                            CString::new("x509privatekey")?.into_raw()
                                as *const ::std::os::raw::c_char,
                            CString::new(connection_info.openssl_private_key.clone())?.into_raw()
                                as *const c_void,
                        )
                {
                    return Err(Box::<dyn Error + Send + Sync>::from(
                        "error while setting x509 cert and key on IotHubClient handle",
                    ));
                }
            }

            self.handle = Some(handle);

            Ok(())
        }
    }

    fn create_from_connection_string(
        &mut self,
        connection_string: CString,
    ) -> Result<(), IotError> {
        unsafe {
            let handle = IoTHubDeviceClient_LL_CreateFromConnectionString(
                connection_string.into_raw(),
                Some(MQTT_Protocol),
            );

            if handle.is_null() {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubDeviceClient_LL_CreateFromConnectionString()",
                ));
            }

            self.handle = Some(handle);

            Ok(())
        }
    }

    fn do_work(&mut self) {
        unsafe {
            IoTHubDeviceClient_LL_DoWork(self.handle.unwrap());
        }
    }

    fn destroy(&mut self) {
        unsafe {
            IoTHubDeviceClient_LL_Destroy(self.handle.unwrap());
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
                self.handle.unwrap(),
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
                    self.handle.unwrap(),
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
                != IoTHubDeviceClient_LL_SetConnectionStatusCallback(
                    self.handle.unwrap(),
                    callback,
                    ctx,
                )
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
                != IoTHubDeviceClient_LL_SetMessageCallback(self.handle.unwrap(), callback, ctx)
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
                != IoTHubDeviceClient_LL_SetDeviceTwinCallback(self.handle.unwrap(), callback, ctx)
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
                != IoTHubDeviceClient_LL_GetTwinAsync(self.handle.unwrap(), callback, ctx)
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
                != IoTHubDeviceClient_LL_SetDeviceMethodCallback(
                    self.handle.unwrap(),
                    callback,
                    ctx,
                )
            {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubDeviceClient_LL_SetDeviceMethodCallback()",
                ));
            }

            Ok(())
        }
    }
}
