use anyhow::Result;
use azure_iot_sdk_sys::*;
use log::{error, info};
use std::collections::HashMap;
use std::ffi::CStr;
use std::ffi::{CString, NulError};
use std::slice;

/// message direction
#[derive(Debug, PartialEq)]
pub enum Direction {
    /// incoming cloud to device (C2D) message
    Incoming,
    /// outgoing device to cloud (D2C) message
    Outgoing,
}

impl Default for Direction {
    fn default() -> Self {
        Direction::Outgoing
    }
}
/// Let's you either create an outgoing D2C messages or parse an incoming cloud to device (C2D) messages.
/// ```rust, no_run
/// # use azure_iot_sdk::client::*;
/// # struct MyEventHandler {}
/// # impl EventHandler for MyEventHandler {}
/// #
/// # let event_handler = MyEventHandler{};
/// # let mut client = IotHubClient::from_identity_service(event_handler).unwrap();
/// #
/// let msg = IotMessage::builder()
///     .set_body(
///         serde_json::to_vec(r#"{"my telemetry message": "hi from device"}"#).unwrap(),
///     )
///     .set_id("my msg id")
///     .set_correlation_id("my correleation id")
///     .set_property(
///         "my property key",
///         "my property value",
///     )
///     .set_output_queue("my output queue")
///     .build()
///     .unwrap();
///
/// client.send_d2c_message(msg);
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
    pub system_properties: HashMap<CString, CString>,
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
            output_queue: String::from("output"),
            ..Default::default()
        }
    }

    pub(crate) fn from_incoming_handle(
        handle: IOTHUB_MESSAGE_HANDLE,
        property_keys: Vec<CString>,
    ) -> Result<Self> {
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

            add_system_property(CString::new("$.mid")?, IoTHubMessage_GetMessageId(handle));
            add_system_property(
                CString::new("$.cid")?,
                IoTHubMessage_GetCorrelationId(handle),
            );
            add_system_property(
                CString::new("$.ct")?,
                IoTHubMessage_GetContentTypeSystemProperty(handle),
            );
            add_system_property(
                CString::new("$.ce")?,
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

            Ok(IotMessage {
                handle: Some(handle),
                body,
                direction: Direction::Incoming,
                output_queue: CString::new("output")?,
                system_properties,
                properties,
            })
        }
    }

    pub(crate) fn create_outgoing_handle(&mut self) -> Result<IOTHUB_MESSAGE_HANDLE> {
        assert_eq!(self.direction, Direction::Outgoing);

        self.destroy_handle();

        unsafe {
            let handle = IoTHubMessage_CreateFromByteArray(self.body.as_ptr(), self.body.len());

            if handle.is_null() {
                anyhow::bail!("error while calling IoTHubMessage_CreateFromByteArray()");
            }

            for (key, value) in &self.system_properties {
                let key = key.to_str()?;
                let res = match key {
                    "$.mid" => IoTHubMessage_SetMessageId(handle, value.as_ptr()),
                    "$.cid" => IoTHubMessage_SetCorrelationId(handle, value.as_ptr()),
                    "$.ct" => IoTHubMessage_SetContentTypeSystemProperty(handle, value.as_ptr()),
                    "$.ce" => {
                        IoTHubMessage_SetContentEncodingSystemProperty(handle, value.as_ptr())
                    }
                    _ => {
                        error!("unknown system property found for key: {}", key);
                        IOTHUB_MESSAGE_RESULT_TAG_IOTHUB_MESSAGE_OK
                    }
                };

                if res != IOTHUB_MESSAGE_RESULT_TAG_IOTHUB_MESSAGE_OK {
                    anyhow::bail!("error while setting system property for key: {}", key);
                }
            }

            for (key, value) in &self.properties {
                if IOTHUB_MESSAGE_RESULT_TAG_IOTHUB_MESSAGE_OK
                    != IoTHubMessage_SetProperty(handle, key.as_ptr(), value.as_ptr())
                {
                    anyhow::bail!(
                        "error while setting property for: {}, {}",
                        key.to_str()?,
                        value.to_str()?
                    );
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
/// ```rust, no_run
/// # use azure_iot_sdk::client::*;
/// # struct MyEventHandler {}
/// # impl EventHandler for MyEventHandler {}
/// #
/// # let event_handler = MyEventHandler{};
/// # let mut client = IotHubClient::from_identity_service(event_handler).unwrap();
/// #
/// let msg = IotMessage::builder()
///     .set_body(
///         serde_json::to_vec(r#"{"my telemetry message": "hi from device"}"#).unwrap(),
///     )
///     .set_id("my msg id")
///     .set_correlation_id("my correleation id")
///     .set_property(
///         "my property key",
///         "my property value",
///     )
///     .set_output_queue("my output queue")
///     .build()
///     .unwrap();
///
/// client.send_d2c_message(msg);
/// ```
#[derive(Debug, Default)]
pub struct IotMessageBuilder {
    message: Option<Vec<u8>>,
    output_queue: String,
    properties: HashMap<String, String>,
    system_properties: HashMap<String, String>,
}

impl IotMessageBuilder {
    /// Set the message body
    /// ```rust, no_run
    /// # use azure_iot_sdk::client::*;
    /// # struct MyEventHandler {}
    /// # impl EventHandler for MyEventHandler {}
    /// #
    /// # let event_handler = MyEventHandler{};
    /// # let mut client = IotHubClient::from_identity_service(event_handler).unwrap();
    /// #
    /// let msg = IotMessage::builder()
    ///     .set_body(
    ///         serde_json::to_vec(r#"{"my telemetry message": "hi from device"}"#).unwrap(),
    ///     )
    ///     .build()
    ///     .unwrap();
    ///
    /// client.send_d2c_message(msg);
    /// ```
    pub fn set_body(mut self, body: Vec<u8>) -> Self {
        self.message = Some(body);
        self
    }

    /// Set the identifier for this message
    /// ```rust, no_run
    /// # use azure_iot_sdk::client::*;
    /// # struct MyEventHandler {}
    /// # impl EventHandler for MyEventHandler {}
    /// #
    /// # let event_handler = MyEventHandler{};
    /// # let mut client = IotHubClient::from_identity_service(event_handler).unwrap();
    /// #
    /// let msg = IotMessage::builder()
    ///     .set_id("my msg id")
    ///     .build()
    ///     .unwrap();
    ///
    /// client.send_d2c_message(msg);
    /// ```
    pub fn set_id(self, mid: impl Into<String>) -> Self {
        self.set_system_property("$.mid", mid)
    }

    /// Set the identifier for this message
    /// ```rust, no_run
    /// # use azure_iot_sdk::client::*;
    /// # struct MyEventHandler {}
    /// # impl EventHandler for MyEventHandler {}
    ///
    /// # let event_handler = MyEventHandler{};
    /// # let mut client = IotHubClient::from_identity_service(event_handler).unwrap();
    /// #
    /// let msg = IotMessage::builder()
    ///     .set_correlation_id("my correlation id")
    ///     .build()
    ///     .unwrap();
    ///
    /// client.send_d2c_message(msg);
    /// ```
    pub fn set_correlation_id(self, cid: impl Into<String>) -> Self {
        self.set_system_property("$.cid", cid)
    }

    /// Set the content-type for this message, such as `text/plain`.
    /// To allow routing query on the message body, this value should be set to `application/json`
    /// ```rust, no_run
    /// # use azure_iot_sdk::client::*;
    /// # struct MyEventHandler {}
    /// # impl EventHandler for MyEventHandler {}
    /// #
    /// # let event_handler = MyEventHandler{};
    /// # let mut client = IotHubClient::from_identity_service(event_handler).unwrap();
    /// #
    /// let msg = IotMessage::builder()
    ///     .set_content_type("application/json")
    ///     .build()
    ///     .unwrap();
    ///
    /// client.send_d2c_message(msg);
    /// ```
    pub fn set_content_type(self, content_type: impl Into<String>) -> Self {
        self.set_system_property("$.ct", content_type)
    }

    /// Set the content-encoding for this message
    /// If the content-type is set to `application/json`, allowed values are `UTF-8`, `UTF-16`, `UTF-32`
    /// To allow routing query on the message body, this value should be set to `application/json`
    /// ```rust, no_run
    /// # use azure_iot_sdk::client::*;
    /// # struct MyEventHandler {}
    /// # impl EventHandler for MyEventHandler {}
    /// #
    /// # let event_handler = MyEventHandler{};
    /// # let mut client = IotHubClient::from_identity_service(event_handler).unwrap();
    /// #
    /// let msg = IotMessage::builder()
    ///     .set_content_encoding("UTF-8")
    ///     .build()
    ///     .unwrap();
    ///
    /// client.send_d2c_message(msg);
    /// ```
    pub fn set_content_encoding(self, content_encoding: impl Into<String>) -> Self {
        self.set_system_property("$.ce", content_encoding)
    }

    /// Set the output queue to be used with this message
    /// ```rust, no_run
    /// # use azure_iot_sdk::client::*;
    /// # struct MyEventHandler {}
    /// # impl EventHandler for MyEventHandler {}
    /// #
    /// # let event_handler = MyEventHandler{};
    /// # let mut client = IotHubClient::from_identity_service(event_handler).unwrap();
    /// #
    /// let msg = IotMessage::builder()
    ///     .set_output_queue("my output queue")
    ///     .build()
    ///     .unwrap();
    ///
    /// client.send_d2c_message(msg);
    /// ```
    pub fn set_output_queue(mut self, queue: impl Into<String>) -> Self {
        self.output_queue = queue.into();
        self
    }

    /// Add a message property
    /// ```rust, no_run
    /// # use azure_iot_sdk::client::*;
    /// # struct MyEventHandler {}
    /// # impl EventHandler for MyEventHandler {}
    /// #
    /// # let event_handler = MyEventHandler{};
    /// # let mut client = IotHubClient::from_identity_service(event_handler).unwrap();
    /// #
    /// let msg = IotMessage::builder()
    ///     .set_property("my key1", "my property1")
    ///     .set_property("my key2", "my property2")
    ///     .build()
    ///     .unwrap();
    ///
    /// client.send_d2c_message(msg);
    /// ```
    pub fn set_property(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.properties.insert(key.into(), value.into());
        self
    }

    /// Build into a message instance
    pub fn build(self) -> Result<IotMessage> {
        Ok(IotMessage {
            handle: None,
            body: self.message.unwrap(),
            direction: Direction::Outgoing,
            output_queue: CString::new(self.output_queue)?,
            properties: self
                .properties
                .into_iter()
                .map(|(k, v)| {
                    let key = CString::new(k)?;
                    let value = CString::new(v)?;
                    Ok((key, value))
                })
                .collect::<Result<HashMap<CString, CString>, NulError>>()?,
            system_properties: self
                .system_properties
                .into_iter()
                .map(|(k, v)| {
                    let key = CString::new(k)?;
                    let value = CString::new(v)?;
                    Ok((key, value))
                })
                .collect::<Result<HashMap<CString, CString>, NulError>>()?,
        })
    }

    /// System properties that are user settable
    /// https://docs.microsoft.com/bs-cyrl-ba/azure/iot-hub/iot-hub-devguide-messages-construct#system-properties-of-d2c-iot-hub-messages
    /// The full list of valid "wire ids" is available here:
    /// https://github.com/Azure/azure-iot-sdk-csharp/blob/67f8c75576edfcbc20e23a01afc88be47552e58c/iothub/device/src/Transport/Mqtt/MqttIotHubAdapter.cs#L1068-L1086
    /// If you need to add support for a new property,
    /// you should create a new public function that sets the appropriate wire id.
    fn set_system_property(
        mut self,
        property_name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.system_properties
            .insert(property_name.into(), value.into());
        self
    }
}
