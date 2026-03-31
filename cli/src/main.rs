use rustls::crypto::{ring::default_provider, CryptoProvider};
use stablesats::app;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = CryptoProvider::install_default(default_provider());
    app::run().await
}
