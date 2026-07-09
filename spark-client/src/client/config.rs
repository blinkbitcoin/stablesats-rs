use breez_sdk_spark::Network;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SparkClientConfig {
    #[serde(default)]
    pub api_key: String,
    pub network: SparkNetwork,
    #[serde(default)]
    pub mnemonic: String,
    #[serde(default)]
    pub mnemonic_passphrase: Option<String>,
    #[serde(default)]
    pub postgres_connection_string: String,
    #[serde(default)]
    pub token_identifier: String,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SparkNetwork {
    #[default]
    Mainnet,
    Regtest,
}

impl From<SparkNetwork> for Network {
    fn from(value: SparkNetwork) -> Self {
        match value {
            SparkNetwork::Mainnet => Network::Mainnet,
            SparkNetwork::Regtest => Network::Regtest,
        }
    }
}

const fn default_page_size() -> u32 {
    100
}

impl SparkClientConfig {
    pub fn page_size(&self) -> u32 {
        self.page_size
    }
}
