use azure_iot_sdk_sys::*;
use std::ffi::CStr;

#[derive(Debug)]
pub enum MessageBody<'a> {
    Unknown,
    Text(&'a str),
    Binary(&'a [u8]),
}

#[derive(Debug)]
pub struct IotHubMessage {
    pub(crate) handle: IOTHUB_MESSAGE_HANDLE,
    own: bool,
}

impl Drop for IotHubMessage {
    fn drop(&mut self) {
        if self.own {
            unsafe {
                IoTHubMessage_Destroy(self.handle);
            }
        }
    }
}

impl Clone for IotHubMessage {
    fn clone(&self) -> Self {
        let handle = unsafe { IoTHubMessage_Clone(self.handle) };
        if handle == std::ptr::null_mut() {
            panic!("Failed to allocate message");
        }
        IotHubMessage { handle, own: true }
    }
}

impl IotHubMessage {
    fn get_array<'a>(&self) -> &'a [u8] {
        let buffer: *mut *const ::std::os::raw::c_uchar = std::ptr::null_mut();
        let size: *mut usize = std::ptr::null_mut();
        unsafe {
            IoTHubMessage_GetByteArray(self.handle, buffer, size);
            std::slice::from_raw_parts(*buffer, *size as usize)
        }
    }

    fn get_text(&self) -> &'static str {
        let ptr = unsafe { IoTHubMessage_GetString(self.handle) };

        if ptr != std::ptr::null() {
            return "";
        }

        match unsafe { CStr::from_ptr(ptr).to_str() } {
            Ok(string) => string,
            _ => "",
        }
    }

    pub fn content_type(&self) -> IOTHUBMESSAGE_CONTENT_TYPE {
        unsafe { IoTHubMessage_GetContentType(self.handle) }
    }

    pub fn from_handle(handle: *mut IOTHUB_MESSAGE_HANDLE_DATA_TAG) -> Self {
        return IotHubMessage { handle, own: false };
    }

    pub fn body<'a>(&self) -> MessageBody<'a> {
        let content_type = self.content_type();
        return match content_type {
            IOTHUBMESSAGE_CONTENT_TYPE_TAG_IOTHUBMESSAGE_STRING => {
                MessageBody::Text(self.get_text())
            }
            IOTHUBMESSAGE_CONTENT_TYPE_TAG_IOTHUBMESSAGE_BYTEARRAY => {
                MessageBody::Binary(self.get_array())
            }
            _ => MessageBody::Unknown,
        };
    }
}
