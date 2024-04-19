use anyhow::Result;
use azure_iot_sdk_sys::*;
use std::ffi::{CStr, CString};

#[cfg(any(feature = "module_client", feature = "edge_client"))]
#[derive(Default, Debug)]
pub struct ModuleTwin {
    handle: Option<IOTHUB_MODULE_CLIENT_HANDLE>,
}

#[cfg(feature = "device_client")]
#[derive(Default, Debug)]
pub struct DeviceTwin {
    handle: Option<IOTHUB_DEVICE_CLIENT_HANDLE>,
}

/// type of client twin
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ClientType {
    /// edge module twin client
    Edge,
    /// module twin client
    Module,
    /// device twin client
    Device,
}

pub(crate) fn sdk_version_string() -> String {
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
    fn create_from_connection_string(&mut self, connection_string: CString) -> Result<()>;

    fn destroy(&mut self);

    fn send_event_to_output_async(
        &self,
        message_handle: IOTHUB_MESSAGE_HANDLE,
        queue: CString,
        callback: IOTHUB_CLIENT_EVENT_CONFIRMATION_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()>;

    fn send_reported_state(
        &self,
        reported_state: CString,
        size: usize,
        callback: IOTHUB_CLIENT_REPORTED_STATE_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()>;

    fn set_connection_status_callback(
        &self,
        callback: IOTHUB_CLIENT_CONNECTION_STATUS_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()>;

    fn set_input_message_callback(
        &self,
        callback: IOTHUB_CLIENT_MESSAGE_CALLBACK_ASYNC,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()>;

    fn set_twin_callback(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_TWIN_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()>;

    fn twin_async(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_TWIN_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()>;

    fn set_method_callback(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_METHOD_CALLBACK_ASYNC,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()>;

    fn set_option(&self, option_name: CString, value: *const std::ffi::c_void) -> Result<()>;

    fn set_retry_policy(
        &self,
        policy: IOTHUB_CLIENT_RETRY_POLICY,
        timeout_secs: usize,
    ) -> Result<()>;
}

#[cfg(feature = "edge_client")]
impl ModuleTwin {
    pub(crate) fn create_from_edge_environment(&mut self) -> Result<()> {
        unsafe {
            let handle = IoTHubModuleClient_CreateFromEnvironment(Some(MQTT_Protocol));

            if handle.is_null() {
                anyhow::bail!("error while calling IoTHubModuleClient_CreateFromEnvironment()");
            }

            self.handle = Some(handle);

            Ok(())
        }
    }
}

#[cfg(any(feature = "module_client", feature = "edge_client"))]
impl Twin for ModuleTwin {
    fn create_from_connection_string(&mut self, connection_string: CString) -> Result<()> {
        unsafe {
            let handle = IoTHubModuleClient_CreateFromConnectionString(
                connection_string.into_raw(),
                Some(MQTT_Protocol),
            );

            if handle.is_null() {
                anyhow::bail!(
                    "error while calling IoTHubModuleClient_CreateFromConnectionString()",
                );
            }

            self.handle = Some(handle);

            Ok(())
        }
    }

    fn destroy(&mut self) {
        unsafe {
            IoTHubModuleClient_Destroy(self.handle.expect("no handle"));
        }
    }

    fn send_event_to_output_async(
        &self,
        message_handle: IOTHUB_MESSAGE_HANDLE,
        queue: CString,
        callback: IOTHUB_CLIENT_EVENT_CONFIRMATION_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_SendEventToOutputAsync(
                    self.handle.expect("no handle"),
                    message_handle,
                    queue.as_ptr(),
                    callback,
                    ctx,
                )
            {
                anyhow::bail!("error while calling IoTHubModuleClient_SendEventToOutputAsync()");
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
    ) -> Result<()> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_SendReportedState(
                    self.handle.expect("no handle"),
                    reported_state.into_raw() as *mut u8,
                    size,
                    callback,
                    ctx,
                )
            {
                anyhow::bail!("error while calling IoTHubModuleClient_SendReportedState()");
            }

            Ok(())
        }
    }

    fn set_connection_status_callback(
        &self,
        callback: IOTHUB_CLIENT_CONNECTION_STATUS_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_SetConnectionStatusCallback(
                    self.handle.expect("no handle"),
                    callback,
                    ctx,
                )
            {
                anyhow::bail!(
                    "error while calling IoTHubModuleClient_SetConnectionStatusCallback()",
                );
            }

            Ok(())
        }
    }

    fn set_input_message_callback(
        &self,
        callback: IOTHUB_CLIENT_MESSAGE_CALLBACK_ASYNC,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()> {
        unsafe {
            let input_name = CString::new("input")?;
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_SetInputMessageCallback(
                    self.handle.expect("no handle"),
                    input_name.as_ptr(),
                    callback,
                    ctx,
                )
            {
                anyhow::bail!("error while calling IoTHubModuleClient_SetInputMessageCallback()");
            }

            Ok(())
        }
    }

    fn set_twin_callback(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_TWIN_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_SetModuleTwinCallback(
                    self.handle.expect("no handle"),
                    callback,
                    ctx,
                )
            {
                anyhow::bail!("error while calling IoTHubModuleClient_SetModuleTwinCallback()");
            }

            Ok(())
        }
    }

    fn twin_async(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_TWIN_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_GetTwinAsync(self.handle.expect("no handle"), callback, ctx)
            {
                anyhow::bail!("error while calling IoTHubModuleClient_GetTwinAsync()");
            }

            Ok(())
        }
    }

    fn set_method_callback(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_METHOD_CALLBACK_ASYNC,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_SetModuleMethodCallback(
                    self.handle.expect("no handle"),
                    callback,
                    ctx,
                )
            {
                anyhow::bail!("error while calling IoTHubModuleClient_SetModuleMethodCallback()");
            }

            Ok(())
        }
    }

    fn set_option(&self, option_name: CString, value: *const std::ffi::c_void) -> Result<()> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubModuleClient_SetOption(
                    self.handle.expect("no handle"),
                    option_name.into_raw(),
                    value,
                )
            {
                anyhow::bail!("error while calling IoTHubModuleClient_SetOption()");
            }

            Ok(())
        }
    }

    fn set_retry_policy(
        &self,
        policy: IOTHUB_CLIENT_RETRY_POLICY,
        timeout_secs: usize,
    ) -> Result<()> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubClient_SetRetryPolicy(
                    self.handle.expect("no handle"),
                    policy,
                    timeout_secs,
                )
            {
                anyhow::bail!("error while calling IoTHubClient_SetRetryPolicy()");
            }

            Ok(())
        }
    }
}

#[cfg(feature = "device_client")]
impl Twin for DeviceTwin {
    fn create_from_connection_string(&mut self, connection_string: CString) -> Result<()> {
        unsafe {
            let handle = IoTHubDeviceClient_CreateFromConnectionString(
                connection_string.into_raw(),
                Some(MQTT_Protocol),
            );

            if handle.is_null() {
                anyhow::bail!(
                    "error while calling IoTHubDeviceClient_CreateFromConnectionString()",
                );
            }

            self.handle = Some(handle);

            Ok(())
        }
    }

    fn destroy(&mut self) {
        unsafe {
            IoTHubDeviceClient_Destroy(self.handle.expect("no handle"));
        }
    }

    fn send_event_to_output_async(
        &self,
        message_handle: IOTHUB_MESSAGE_HANDLE,
        _queue: CString,
        callback: IOTHUB_CLIENT_EVENT_CONFIRMATION_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()> {
        unsafe {
            let result = IoTHubDeviceClient_SendEventAsync(
                self.handle.expect("no handle"),
                message_handle,
                callback,
                ctx,
            );

            if result != IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK {
                anyhow::bail!("error while calling IoTHubDeviceClient_SendEventAsync()");
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
    ) -> Result<()> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubDeviceClient_SendReportedState(
                    self.handle.expect("no handle"),
                    reported_state.into_raw() as *mut u8,
                    size,
                    callback,
                    ctx,
                )
            {
                anyhow::bail!("error while calling IoTHubDeviceClient_SendReportedState()");
            }

            Ok(())
        }
    }

    fn set_connection_status_callback(
        &self,
        callback: IOTHUB_CLIENT_CONNECTION_STATUS_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubDeviceClient_SetConnectionStatusCallback(
                    self.handle.expect("no handle"),
                    callback,
                    ctx,
                )
            {
                anyhow::bail!(
                    "error while calling IoTHubDeviceClient_SetConnectionStatusCallback()",
                );
            }

            Ok(())
        }
    }

    fn set_input_message_callback(
        &self,
        callback: IOTHUB_CLIENT_MESSAGE_CALLBACK_ASYNC,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubDeviceClient_SetMessageCallback(
                    self.handle.expect("no handle"),
                    callback,
                    ctx,
                )
            {
                anyhow::bail!("error while calling IoTHubDeviceClient_SetMessageCallback()");
            }

            Ok(())
        }
    }

    fn set_twin_callback(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_TWIN_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubDeviceClient_SetDeviceTwinCallback(
                    self.handle.expect("no handle"),
                    callback,
                    ctx,
                )
            {
                anyhow::bail!("error while calling IoTHubDeviceClient_SetDeviceTwinCallback()");
            }

            Ok(())
        }
    }

    fn twin_async(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_TWIN_CALLBACK,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubDeviceClient_GetTwinAsync(self.handle.expect("no handle"), callback, ctx)
            {
                anyhow::bail!("error while calling IoTHubDeviceClient_GetTwinAsync()");
            }

            Ok(())
        }
    }

    fn set_method_callback(
        &self,
        callback: IOTHUB_CLIENT_DEVICE_METHOD_CALLBACK_ASYNC,
        ctx: *mut std::ffi::c_void,
    ) -> Result<()> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubDeviceClient_SetDeviceMethodCallback(
                    self.handle.expect("no handle"),
                    callback,
                    ctx,
                )
            {
                anyhow::bail!("error while calling IoTHubDeviceClient_SetDeviceMethodCallback()");
            }

            Ok(())
        }
    }

    fn set_option(&self, option_name: CString, value: *const std::ffi::c_void) -> Result<()> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubDeviceClient_SetOption(
                    self.handle.expect("no handle"),
                    option_name.into_raw(),
                    value,
                )
            {
                anyhow::bail!("error while calling IoTHubDeviceClient_SetOption()");
            }

            Ok(())
        }
    }

    fn set_retry_policy(
        &self,
        policy: IOTHUB_CLIENT_RETRY_POLICY,
        timeout_secs: usize,
    ) -> Result<()> {
        unsafe {
            if IOTHUB_CLIENT_RESULT_TAG_IOTHUB_CLIENT_OK
                != IoTHubDeviceClient_SetRetryPolicy(
                    self.handle.expect("no handle"),
                    policy,
                    timeout_secs,
                )
            {
                anyhow::bail!("error while calling IoTHubDeviceClient_SetRetryPolicy()");
            }

            Ok(())
        }
    }
}
