//! Let's you create either a D2C messages or parse a C2D messages.
use crate::client::IotError;
use azure_iot_sdk_sys::*;
use log::{error, info};
use std::boxed::Box;
use std::collections::HashMap;
use std::error::Error;
use std::ffi::CStr;
use std::ffi::CString;
use std::slice;

/// message direction
#[derive(Debug, PartialEq)]
pub enum Direction {
    /// incoming C2D message
    Incoming,
    /// outgoing D2C message
    Outgoing,
}

impl Default for Direction {
    fn default() -> Self {
        Direction::Outgoing
    }
}

/// message instance. either representing incoming C2D message or an outgoing D2C message
/// ```rust
/// use azure_iot_sdk::client::*;
/// struct MyEventHandler {}
/// impl EventHandler for MyEventHandler {}
///
/// fn main() {
///     let event_handler = MyEventHandler{};
///     let mut client = IotHubClient::from_identity_service(TwinType::Module, event_handler).unwrap();
/// 
///     let msg = IotMessage::builder()
///         .set_body(
///             serde_json::to_vec(r#"{"my telemetry message": "hi from device"}"#).unwrap(),
///         )
///         .set_id(String::from("my msg id"))
///         .set_correlation_id(String::from("my correleation id"))
///         .set_property(
///             String::from("my property key"),
///             String::from("my property value"),
///         )
///         .set_output_queue(String::from("my output queue"))
///         .build();
/// 
///     client.send_d2c_message(msg).unwrap();
///
///     loop {
///         client.do_work();
///     }
/// }
/// ```
#[derive(Default, Debug)]
pub struct IotMessage {
    handle: Option<IOTHUB_MESSAGE_HANDLE>,
    /// message body
    pub body: Vec<u8>,
    /// output queue name. default: "output"
    pub output_queue: CString,
    /// message direction
    pub direction: Direction,
    /// map of [mqtt message properties](https://docs.microsoft.com/de-de/azure/iot-hub/iot-c-sdk-ref/iothub-message-h/iothubmessage-getproperty)
    pub properties: HashMap<CString, CString>,
    /// map of [mqtt system message properties](https://docs.microsoft.com/de-de/azure/iot-hub/iot-c-sdk-ref/iothub-message-h/iothubmessage-getcontenttypesystemproperty)
    pub system_properties: HashMap<&'static str, CString>,
}

unsafe impl Send for IotMessage {}
unsafe impl Sync for IotMessage {}

impl Drop for IotMessage {
    fn drop(&mut self) {
        if self.direction == Direction::Outgoing {
            self.destroy_handle()
        }
    }
}

impl IotMessage {
    /// Get a builder instance for building up a message
    pub fn builder() -> IotMessageBuilder {
        IotMessageBuilder {
            output_queue: CString::new("output").unwrap(),
            ..Default::default()
        }
    }

    pub(crate) fn from_incoming_handle(
        handle: IOTHUB_MESSAGE_HANDLE,
        property_keys: Vec<CString>,
    ) -> Self {
        unsafe {
            let mut buf_ptr: *const ::std::os::raw::c_uchar = std::ptr::null_mut();
            let mut buf_size: usize = 0;
            let mut system_properties = HashMap::new();
            let mut properties: HashMap<CString, CString> = HashMap::new();
            let body = match IoTHubMessage_GetByteArray(handle, &mut buf_ptr, &mut buf_size) {
                IOTHUB_MESSAGE_RESULT_TAG_IOTHUB_MESSAGE_OK => {
                    if buf_ptr.is_null() {
                        error!("IoTHubMessage_GetByteArray: received invalid body pointer");
                        Vec::new()
                    } else {
                        slice::from_raw_parts(buf_ptr, buf_size as usize).to_vec()
                    }
                }
                _ => {
                    error!("IoTHubMessage_GetByteArray: error while parsing body");
                    Vec::new()
                }
            };

            let mut add_system_property = |key, value: *const ::std::os::raw::c_char| {
                if value.is_null() == false {
                    system_properties.insert(key, CStr::from_ptr(value).to_owned());
                }
            };

            add_system_property("$.mid", IoTHubMessage_GetMessageId(handle));
            add_system_property("$.cid", IoTHubMessage_GetCorrelationId(handle));
            add_system_property("$.ct", IoTHubMessage_GetContentTypeSystemProperty(handle));
            add_system_property(
                "$.ce",
                IoTHubMessage_GetContentEncodingSystemProperty(handle),
            );

            for k in property_keys {
                let v = IoTHubMessage_GetProperty(handle, k.as_ptr());

                if v.is_null() {
                    info!("IoTHubMessage_GetProperty: no value found for: {:?}", k);
                } else {
                    properties.insert(k, CStr::from_ptr(v).to_owned());
                }
            }

            IotMessage {
                handle: Some(handle),
                body,
                direction: Direction::Incoming,
                output_queue: CString::new("output").unwrap(),
                system_properties,
                properties,
            }
        }
    }

    pub(crate) fn create_outgoing_handle(&mut self) -> Result<IOTHUB_MESSAGE_HANDLE, IotError> {
        assert_eq!(self.direction, Direction::Outgoing);

        self.destroy_handle();

        unsafe {
            let handle = IoTHubMessage_CreateFromByteArray(self.body.as_ptr(), self.body.len());

            if handle.is_null() {
                return Err(Box::<dyn Error + Send + Sync>::from(
                    "error while calling IoTHubMessage_CreateFromByteArray()",
                ));
            }

            for (&key, value) in &self.system_properties {
                let res = match key {
                    "$.mid" => IoTHubMessage_SetMessageId(handle, value.as_ptr()),
                    "$.cid" => IoTHubMessage_SetCorrelationId(handle, value.as_ptr()),
                    "$.ct" => IoTHubMessage_SetContentTypeSystemProperty(handle, value.as_ptr()),
                    "$.ce" => {
                        IoTHubMessage_SetContentEncodingSystemProperty(handle, value.as_ptr())
                    }
                    _ => {
                        error!(
                            "unknown system property found: {}, {}",
                            key,
                            value.to_str().unwrap()
                        );
                        IOTHUB_MESSAGE_RESULT_TAG_IOTHUB_MESSAGE_OK
                    }
                };

                if res != IOTHUB_MESSAGE_RESULT_TAG_IOTHUB_MESSAGE_OK {
                    return Err(Box::<dyn Error + Send + Sync>::from(format!(
                        "error while setting system property for: {}, {}",
                        key,
                        value.to_str().unwrap()
                    )));
                }
            }

            for (key, value) in &self.properties {
                if IOTHUB_MESSAGE_RESULT_TAG_IOTHUB_MESSAGE_OK
                    != IoTHubMessage_SetProperty(handle, key.as_ptr(), value.as_ptr())
                {
                    return Err(Box::<dyn Error + Send + Sync>::from(format!(
                        "error while setting property for: {}, {}",
                        key.to_str().unwrap(),
                        value.to_str().unwrap()
                    )));
                }
            }

            self.handle = Some(handle);
        }

        Ok(self.handle.unwrap())
    }

    fn destroy_handle(&mut self) {
        if let Some(handle) = self.handle {
            unsafe {
                IoTHubMessage_Destroy(handle);

                self.handle = None;
            }
        }
    }
}

/// Builder for constructing outgoing D2C message instances
#[derive(Debug, Default)]
pub struct IotMessageBuilder {
    message: Option<Vec<u8>>,
    output_queue: CString,
    properties: HashMap<CString, CString>,
    system_properties: HashMap<&'static str, CString>,
}

impl IotMessageBuilder {
    /// Set the message body
    pub fn set_body(mut self, body: Vec<u8>) -> Self {
        self.message = Some(body);
        self
    }

    /// Set the identifier for this message
    pub fn set_id(self, mid: String) -> Self {
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

    /// Set the output queue to be used with this message.
    pub fn set_output_queue(mut self, queue: String) -> Self {
        self.output_queue = CString::new(queue).unwrap();
        self
    }

    /// Add a message property
    pub fn set_property(mut self, key: String, value: String) -> Self {
        self.properties
            .insert(CString::new(key).unwrap(), CString::new(value).unwrap());
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
    fn set_system_property(mut self, property_name: &'static str, value: String) -> Self {
        self.system_properties
            .insert(property_name, CString::new(value).unwrap());
        self
    }
}
