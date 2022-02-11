use azure_iot_sdk_sys::*;
use std::boxed::Box;
use std::collections::HashMap;
use std::error::Error;
use std::ffi::CString;

#[derive(Debug, PartialEq)]
pub enum Direction {
    Incoming,
    Outgoing,
}

impl Default for Direction {
    fn default() -> Self {
        Direction::Outgoing
    }
}

/// Message used in body of communication
#[derive(Default, Debug)]
pub struct IotMessage {
    handle: Option<IOTHUB_MESSAGE_HANDLE>,
    body: Vec<u8>,
    output_queue: String,
    direction: Direction,
    properties: HashMap<String, String>,
    system_properties: HashMap<String, CString>,
}

unsafe impl Send for IotMessage {}
unsafe impl Sync for IotMessage {}

impl Drop for IotMessage {
    fn drop(&mut self) {
        if let Some(handle) = self.handle {
            unsafe {
                IoTHubMessage_Destroy(handle);
            }
        }
    }
}

/* impl Clone for IotMessage {
    fn clone(&self) -> Self {
        let handle = unsafe { IoTHubMessage_Clone(self.handle) };
        if handle == std::ptr::null_mut() {
            panic!("Failed to allocate message");
        }
        IotMessage { handle, own: true }
    }
} */

impl IotMessage {
    /// Get a builder instance for building up a message
    pub fn builder() -> IotMessageBuilder {
        IotMessageBuilder {
            output_queue: "output".to_string(),
            ..Default::default()
        }
    }

    /// Create an instance from an incoming C2D message
    pub fn from_incoming_handle(_handle: IOTHUB_MESSAGE_HANDLE) -> Self {
        todo!("implement with a C2D device twin example, since module twins are not supported for such a scenario.");
        // use IoTHubMessage_Get... functions to create a message instance
        /*
        IoTHubMessage_GetMessageId(handle, "MSG_ID".);
        IoTHubMessage_GetCorrelationId(handle, "CORE_ID");
        IoTHubMessage_GetContentTypeSystemProperty
        IoTHubMessage_GetContentEncodingSystemProperty
        IoTHubMessage_GetProperty
        */
    }

    /// Create
    pub fn create_outgoing_handle(
        &mut self,
    ) -> Result<IOTHUB_MESSAGE_HANDLE, Box<dyn Error + Send + Sync>> {
        assert_eq!(self.direction, Direction::Outgoing);

        if None == self.handle {
            unsafe {
                let handle = IoTHubMessage_CreateFromByteArray(self.body.as_ptr(), self.body.len());

                if handle.is_null() {
                    return Err(Box::<dyn Error + Send + Sync>::from(
                        "error while calling IoTHubMessage_CreateFromByteArray()",
                    ));
                }

                if let Some(id) = self.system_properties.get("$.mid") {
                    if IOTHUB_MESSAGE_RESULT_TAG_IOTHUB_MESSAGE_OK
                        != IoTHubMessage_SetMessageId(handle, id.as_ptr())
                    {
                        return Err(Box::<dyn Error + Send + Sync>::from(
                            "error while calling IoTHubMessage_SetMessageId()",
                        ));
                    }
                }

                if let Some(id) = self.system_properties.get("$.cid") {
                    if IOTHUB_MESSAGE_RESULT_TAG_IOTHUB_MESSAGE_OK
                        != IoTHubMessage_SetCorrelationId(handle, id.as_ptr())
                    {
                        return Err(Box::<dyn Error + Send + Sync>::from(
                            "error while calling IoTHubMessage_SetCorrelationId()",
                        ));
                    }
                }
                /*
                IoTHubMessage_SetContentTypeSystemProperty
                IoTHubMessage_SetContentEncodingSystemProperty
                IoTHubMessage_SetProperty
                */

                self.handle = Some(handle);
            }
        }

        Ok(self.handle.unwrap())
    }

    pub fn get_output_queue(&self) -> CString {
        CString::new(self.output_queue.clone()).unwrap()
    }
}

/// Builder for constructing Message instances
#[derive(Debug, Default)]
pub struct IotMessageBuilder {
    message: Option<Vec<u8>>,
    output_queue: String,
    properties: HashMap<String, String>,
    system_properties: HashMap<String, CString>,
}

impl IotMessageBuilder {
    /// Set the message body
    pub fn set_body(mut self, body: Vec<u8>) -> Self {
        self.message = Some(body);
        self
    }

    /// Set the identifier for this message
    pub fn set_message_id(self, mid: String) -> Self {
        self.set_system_property("$.mid", mid)
    }

    /// Set the identifier for this message
    pub fn set_correlation_id(self, cid: String) -> Self {
        self.set_system_property("$.cid", cid)
    }

    /// Set the content-type for this message, such as `text/plain`.
    /// To allow routing query on the message body, this value should be set to `application/json`
    pub fn set_content_type(self, content_type: String) -> Self {
        self.set_system_property("$.ct", content_type)
    }

    /// Set the content-encoding for this message.
    /// If the content-type is set to `application/json`, allowed values are `UTF-8`, `UTF-16`, `UTF-32`.
    pub fn set_content_encoding(self, content_encoding: String) -> Self {
        self.set_system_property("$.ce", content_encoding)
    }

    pub fn set_output_queue(mut self, queue: String) -> Self {
        self.output_queue = queue;
        self
    }

    /// Add a message property
    pub fn set_message_property(mut self, key: &str, value: String) -> Self {
        self.properties.insert(key.to_owned(), value);
        self
    }

    /// Build into a message instance
    pub fn build(self) -> IotMessage {
        IotMessage {
            handle: None,
            body: self.message.unwrap(),
            direction: Direction::Outgoing,
            output_queue: self.output_queue,
            properties: self.properties,
            system_properties: self.system_properties,
        }
    }

    /// System properties that are user settable
    /// https://docs.microsoft.com/bs-cyrl-ba/azure/iot-hub/iot-hub-devguide-messages-construct#system-properties-of-d2c-iot-hub-messages
    /// The full list of valid "wire ids" is availabe here:
    /// https://github.com/Azure/azure-iot-sdk-csharp/blob/67f8c75576edfcbc20e23a01afc88be47552e58c/iothub/device/src/Transport/Mqtt/MqttIotHubAdapter.cs#L1068-L1086
    /// If you need to add support for a new property,
    /// you should create a new public function that sets the appropriate wire id.
    fn set_system_property(mut self, property_name: &str, value: String) -> Self {
        self.system_properties
            .insert(property_name.to_owned(), CString::new(value).unwrap());
        self
    }
}
