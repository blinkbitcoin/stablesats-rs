use thiserror::Error;

#[derive(Error, Debug)]
pub enum SparkClientError {
    #[error("SparkClientError - SDK: {0}")]
    Sdk(#[from] breez_sdk_spark::SdkError),
    #[error("SparkClientError - Serde: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("SparkClientError - CursorDecode: {0}")]
    CursorDecode(#[from] data_encoding::DecodeError),
    #[error("SparkClientError - Authentication: {0}")]
    Authentication(String),
    #[error("SparkClientError - Config: {0}")]
    Config(String),
    #[error("SparkClientError - Cursor: {0}")]
    Cursor(String),
    #[error("SparkClientError - InvalidTimestamp: {0}")]
    InvalidTimestamp(u64),
}
