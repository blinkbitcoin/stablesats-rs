#![cfg_attr(feature = "fail-on-warnings", deny(warnings))]
#![cfg_attr(feature = "fail-on-warnings", deny(clippy::all))]

mod client;

pub use client::*;

#[cfg(feature = "test-support")]
pub mod test_support;
