pub mod client;
pub mod message;
pub mod twin;
use std::error::Error;

pub type IotError = Box<dyn Error + Send + Sync>;
