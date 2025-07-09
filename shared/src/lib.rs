#![cfg_attr(feature = "fail-on-warnings", deny(warnings))]
#![cfg_attr(feature = "fail-on-warnings", deny(clippy::all))]

pub mod health;
pub mod macros;
pub mod payload;
pub mod pubsub;
pub mod sqlxmq;
pub mod test_utils;
pub mod time;
pub mod tracing;

#[derive(Debug)]
pub struct ParseIdError(pub &'static str);
