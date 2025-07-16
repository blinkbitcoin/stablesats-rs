mod tick;

use futures::{SinkExt, Stream, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

pub use crate::error::*;
pub use tick::*;

pub(crate) const OKEX_WS_URL: &str = "wss://ws.okx.com:8443/ws/v5/public";

pub async fn subscribe_btc_usd_swap_price_tick(
) -> Result<std::pin::Pin<Box<dyn Stream<Item = OkexPriceTick> + Send>>, PriceFeedError> {
    let _ = Url::parse(OKEX_WS_URL).expect("invalid okex_ws_url");
    let request = OKEX_WS_URL.into_client_request()?;
    let (ws_stream, _) = connect_async(request).await?;
    let (mut sender, receiver) = ws_stream.split();

    let subscribe_args = serde_json::json!({
        "op": "subscribe",
        "args": [
           {
                "channel": "tickers",
                "instId": "BTC-USD-SWAP"
            }
        ]
    })
    .to_string();
    let item = Message::from(subscribe_args);

    sender.send(item).await?;

    Ok(Box::pin(receiver.filter_map(|message| async {
        if let Ok(msg) = message {
            if let Ok(msg_str) = msg.into_text() {
                if let Ok(tick) = serde_json::from_str::<OkexPriceTick>(&msg_str) {
                    return Some(tick);
                }
            }
        }
        None
    })))
}
