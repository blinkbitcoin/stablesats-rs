use std::env;

use spark_client::*;

async fn configured_client() -> anyhow::Result<SparkClient> {
    let api_key = env::var("SPARK_API_KEY")?;
    let network = match env::var("SPARK_NETWORK")?.to_lowercase().as_str() {
        "mainnet" => SparkNetwork::Mainnet,
        "regtest" => SparkNetwork::Regtest,
        other => anyhow::bail!("Unsupported SPARK_NETWORK: {other}"),
    };
    let mnemonic = env::var("SPARK_MNEMONIC")?;
    let mnemonic_passphrase = env::var("SPARK_MNEMONIC_PASSPHRASE").ok();
    let token_identifier = env::var("SPARK_TOKEN_IDENTIFIER")?;
    let postgres_connection_string = env::var("SPARK_POSTGRES_URL")?;

    let client = SparkClient::connect(SparkClientConfig {
        api_key,
        network,
        mnemonic,
        mnemonic_passphrase,
        postgres_connection_string,
        token_identifier,
        page_size: 100,
    })
    .await?;

    Ok(client)
}

#[tokio::test]
async fn transactions_list() -> anyhow::Result<()> {
    if let Ok(client) = configured_client().await {
        let transactions = client.transactions_list(None).await?;
        assert!(transactions.list.len() <= 100);
    }
    Ok(())
}
