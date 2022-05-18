#![warn(missing_docs)]

//! Wrapper around azure iot-c-sdk-ref.
//! 
//! A reference implementation can be found [here](https://github.com/ICS-DeviceManagement/iot-client-template-rs).//! 
//!
//! Provides an abstraction over Microsoft's iot-c-sdk in order to develop device- and module twin client applications.
//! All API's exposed by this crate base on the following low level function interfaces:
//! - [module client](https://docs.microsoft.com/de-de/azure/iot-hub/iot-c-sdk-ref/iothub-module-client-ll-h)
//! - [device client](https://docs.microsoft.com/de-de/azure/iot-hub/iot-c-sdk-ref/iothub-device-client-ll-h)
//! 
//! The crate must be configured via cargo feature to one of the following twin types:
//! - device_twin
//! - module_twin
//! - edge_twin
//!
//! The following use cases can be realized by using this crate:
//! - connect as iot module, iot edge module or device client to azure iothub.
//!     - use connection string directly
//!     - use iot-identity-service (only for iot modules and device clients) (must be installed on device) to create the client and build the connection string. ***Note***:
//!       This feature is currently [not supported for all combinations of identity type and authentication mechanism](https://azure.github.io/iot-identity-service/develop-an-agent.html#connecting-your-agent-to-iot-hub).
//!     - use iot edge environment (only for iot edge module)
//! - [module twin](https://docs.microsoft.com/en-us/azure/iot-hub/iot-hub-devguide-module-twins) or [device twin](https://docs.microsoft.com/en-us/azure/iot-hub/iot-hub-devguide-device-twins) twin based communication with iothub:
//!     - read/write tags
//!     - receive desired properties
//!     - send reported properties
//! - [direct methods](https://docs.microsoft.com/en-us/azure/iot-hub/iot-hub-devguide-direct-methods)
//! - [device to cloud (D2C) messages](https://docs.microsoft.com/en-us/azure/iot-hub/iot-hub-devguide-messages-d2c)
//! - [cloud to device (C2D) messages](https://docs.microsoft.com/en-us/azure/iot-hub/iot-hub-devguide-messages-c2d)

/// iothub client
pub mod client;
