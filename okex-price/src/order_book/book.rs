use itertools::Itertools;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use shared::{payload::*, time::*};
use std::collections::BTreeMap;

use crate::{ChannelArgs, PriceFeedError};

const CHECKSUM_DEPTH_LIMIT: usize = 25;
const CENTS_PER_OKEX_SWAP_CONTRACT: Decimal = dec!(10000);

#[derive(Debug, Deserialize, PartialEq, Eq, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderBookAction {
    Snapshot,
    Update,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Clone, Serialize)]
#[serde(from = "PriceQuantityRaw")]
pub struct PriceQuantity {
    pub price: Decimal,
    pub quantity: Decimal,
}

#[derive(Debug, Deserialize)]
#[serde(transparent)]
struct PriceQuantityRaw(Vec<Decimal>);

impl From<PriceQuantityRaw> for PriceQuantity {
    fn from(raw: PriceQuantityRaw) -> Self {
        let mut iter = raw.0.into_iter();
        let price = iter
            .next()
            .expect("Missing price element of order book price array");
        let quantity = iter
            .next()
            .expect("Missing quantity element of order book price array");
        Self { price, quantity }
    }
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct OrderBookChannelData {
    pub asks: Vec<PriceQuantity>,
    pub bids: Vec<PriceQuantity>,
    pub ts: TimeStampMilliStr,
    pub checksum: i32,
    #[serde(default, rename = "prevSeqId")]
    pub prev_seq_id: Option<i64>,
    #[serde(default, rename = "seqId")]
    pub seq_id: Option<i64>,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct OkexOrderBook {
    pub arg: ChannelArgs,
    pub action: OrderBookAction,
    pub data: Vec<OrderBookChannelData>,
}

#[derive(Debug, PartialOrd, PartialEq, Eq, Ord, Clone, Deserialize)]
#[serde(transparent)]
pub struct OrderPrice(Decimal);
impl From<Decimal> for OrderPrice {
    fn from(d: Decimal) -> Self {
        Self(d)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct OrderBookIncrement {
    pub asks: BTreeMap<OrderPrice, Decimal>,
    pub bids: BTreeMap<OrderPrice, Decimal>,
    pub timestamp: TimeStamp,
    pub new_checksum: i32,
    #[serde(default)]
    pub previous_sequence_id: Option<i64>,
    #[serde(default)]
    pub sequence_id: Option<i64>,
    pub action: OrderBookAction,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CompleteOrderBook {
    asks: BTreeMap<OrderPrice, Decimal>,
    bids: BTreeMap<OrderPrice, Decimal>,
    timestamp: TimeStamp,
    checksum: i32,
    sequence_id: Option<i64>,
}
impl TryFrom<OrderBookIncrement> for CompleteOrderBook {
    type Error = PriceFeedError;
    fn try_from(book: OrderBookIncrement) -> Result<Self, Self::Error> {
        if book.new_checksum == 0 && book.sequence_id.is_none() {
            return Err(PriceFeedError::SequenceValidation);
        }

        let result = CompleteOrderBook {
            asks: book.asks,
            bids: book.bids,
            timestamp: book.timestamp,
            checksum: book.new_checksum,
            sequence_id: book.sequence_id,
        };
        result.verify_checksum()?;
        Ok(result)
    }
}

impl CompleteOrderBook {
    #[allow(clippy::result_large_err)]
    fn verify_checksum(&self) -> Result<(), PriceFeedError> {
        if self.checksum == 0 {
            return Ok(());
        }

        let cs_res = self.calculate_checksum();
        if cs_res != self.checksum {
            return Err(PriceFeedError::CheckSumValidation);
        }
        Ok(())
    }

    #[allow(clippy::result_large_err)]
    fn try_merge(&self, increment: OrderBookIncrement) -> Result<Self, PriceFeedError> {
        if increment.new_checksum == 0 {
            self.verify_sequence(&increment)?;
        }

        let new_book = match increment.action {
            OrderBookAction::Snapshot => CompleteOrderBook::try_from(increment)?,
            OrderBookAction::Update => {
                let mut new_book = CompleteOrderBook {
                    timestamp: increment.timestamp,
                    checksum: increment.new_checksum,
                    sequence_id: increment.sequence_id,
                    ..self.clone()
                };

                for (ask_price, ask_qty) in increment.asks {
                    if ask_qty == Decimal::ZERO {
                        new_book.asks.remove(&ask_price);
                    } else {
                        new_book.asks.insert(ask_price, ask_qty);
                    }
                }

                for (bid_price, bid_qty) in increment.bids {
                    if bid_qty == Decimal::ZERO {
                        new_book.bids.remove(&bid_price);
                    } else {
                        new_book.bids.insert(bid_price, bid_qty);
                    }
                }

                new_book
            }
        };

        new_book.verify_checksum()?;

        Ok(new_book)
    }

    fn calculate_checksum(&self) -> i32 {
        let asks_list = self
            .asks
            .iter()
            .enumerate()
            .filter_map(|(idx, (price, qty))| {
                if idx < CHECKSUM_DEPTH_LIMIT {
                    Some(format!("{}:{}", price.0, qty))
                } else {
                    None
                }
            })
            .collect::<Vec<String>>();

        let bids_list = self
            .bids
            .iter()
            .rev()
            .enumerate()
            .take_while(|(index, _)| index < &CHECKSUM_DEPTH_LIMIT)
            .map(|(_, (price, qty))| format!("{}:{}", price.0, qty));

        let crc = Itertools::intersperse(bids_list.interleave(asks_list), ":".to_string())
            .collect::<String>();

        crc32fast::hash(crc.as_bytes()) as i32
    }

    fn verify_sequence(&self, increment: &OrderBookIncrement) -> Result<(), PriceFeedError> {
        match (
            self.sequence_id,
            increment.previous_sequence_id,
            increment.sequence_id,
        ) {
            (Some(current), Some(previous), Some(next)) => {
                if current == previous && next >= previous {
                    Ok(())
                } else {
                    Err(PriceFeedError::SequenceValidation)
                }
            }
            _ => Err(PriceFeedError::SequenceValidation),
        }
    }
}

impl TryFrom<CompleteOrderBook> for OrderBookPayload {
    type Error = PriceFeedError;

    fn try_from(book: CompleteOrderBook) -> Result<Self, Self::Error> {
        if book.asks.is_empty() || book.bids.is_empty() {
            return Err(PriceFeedError::EmptyBookSide);
        }
        let mut asks_map = BTreeMap::new();
        for (ask_price, ask_qty) in book.asks {
            let price = PriceRatioRaw::from_one_btc_in_usd_price(ask_price.0).numerator_amount();
            let _ = asks_map.insert(
                PriceRaw::from(price),
                VolumeInCentsRaw::from(ask_qty * CENTS_PER_OKEX_SWAP_CONTRACT),
            );
        }

        let mut bids_map = BTreeMap::new();
        for (bid_price, bid_qty) in book.bids {
            let price = PriceRatioRaw::from_one_btc_in_usd_price(bid_price.0).numerator_amount();
            let _ = bids_map.insert(
                PriceRaw::from(price),
                VolumeInCentsRaw::from(bid_qty * CENTS_PER_OKEX_SWAP_CONTRACT),
            );
        }

        Ok(Self {
            asks: asks_map,
            bids: bids_map,
            timestamp: book.timestamp,
            exchange: ExchangeIdRaw::from(OKEX_EXCHANGE_ID),
        })
    }
}

#[derive(Clone)]
pub struct OrderBookCache {
    current: CompleteOrderBook,
}

impl OrderBookCache {
    pub fn new(book: CompleteOrderBook) -> Self {
        Self { current: book }
    }

    #[allow(clippy::result_large_err)]
    pub fn update_order_book(&mut self, book: OrderBookIncrement) -> Result<(), PriceFeedError> {
        self.current = self.current.try_merge(book)?;
        Ok(())
    }

    pub fn latest(&self) -> &CompleteOrderBook {
        &self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn load_order_book(filename: &str) -> anyhow::Result<OkexOrderBook> {
        let contents = fs::read_to_string(format!("./tests/fixtures/order-book-{}.json", filename))
            .expect(&format!("Couldn't load fixture {}", filename));

        let res = serde_json::from_str::<OkexOrderBook>(&contents)?;
        Ok(res)
    }

    #[test]
    fn merge() -> anyhow::Result<()> {
        let snapshot = load_order_book("snapshot")?;

        let order_book_incr = OrderBookIncrement::try_from(snapshot)?;
        let mut cache = OrderBookCache::new(order_book_incr.try_into()?);

        let update_1 = load_order_book("update-1")?;
        let incr_1 = OrderBookIncrement::try_from(update_1)?;
        assert!(cache.update_order_book(incr_1.clone()).is_ok());
        assert_eq!(cache.latest().checksum, incr_1.new_checksum);

        let update_2 = load_order_book("update-2")?;
        let incr_2 = OrderBookIncrement::try_from(update_2)?;
        assert!(cache.update_order_book(incr_2.clone()).is_ok());
        assert_eq!(cache.latest().checksum, incr_2.new_checksum);

        let update_3 = load_order_book("update-3")?;
        let incr_3 = OrderBookIncrement::try_from(update_3)?;
        assert!(cache.update_order_book(incr_3.clone()).is_ok());
        assert_eq!(cache.latest().checksum, incr_3.new_checksum);

        Ok(())
    }

    #[test]
    fn merge_with_zero_checksums_uses_sequence_ids() -> anyhow::Result<()> {
        let snapshot = load_order_book("snapshot")?;

        let mut order_book_incr = OrderBookIncrement::try_from(snapshot)?;
        order_book_incr.new_checksum = 0;
        order_book_incr.previous_sequence_id = Some(-1);
        order_book_incr.sequence_id = Some(10);
        let mut cache = OrderBookCache::new(order_book_incr.try_into()?);

        let update_1 = load_order_book("update-1")?;
        let mut incr_1 = OrderBookIncrement::try_from(update_1)?;
        incr_1.new_checksum = 0;
        incr_1.previous_sequence_id = Some(10);
        incr_1.sequence_id = Some(11);
        assert!(cache.update_order_book(incr_1.clone()).is_ok());
        assert_eq!(cache.latest().checksum, incr_1.new_checksum);
        assert_eq!(cache.latest().sequence_id, incr_1.sequence_id);

        let update_2 = load_order_book("update-2")?;
        let mut incr_2 = OrderBookIncrement::try_from(update_2)?;
        incr_2.new_checksum = 0;
        incr_2.previous_sequence_id = Some(11);
        incr_2.sequence_id = Some(12);
        assert!(cache.update_order_book(incr_2.clone()).is_ok());
        assert_eq!(cache.latest().checksum, incr_2.new_checksum);
        assert_eq!(cache.latest().sequence_id, incr_2.sequence_id);

        Ok(())
    }

    #[test]
    fn merge_with_zero_checksums_rejects_sequence_gaps() -> anyhow::Result<()> {
        let snapshot = load_order_book("snapshot")?;

        let mut order_book_incr = OrderBookIncrement::try_from(snapshot)?;
        order_book_incr.new_checksum = 0;
        order_book_incr.previous_sequence_id = Some(-1);
        order_book_incr.sequence_id = Some(10);
        let mut cache = OrderBookCache::new(order_book_incr.try_into()?);

        let update_1 = load_order_book("update-1")?;
        let mut incr_1 = OrderBookIncrement::try_from(update_1)?;
        incr_1.new_checksum = 0;
        incr_1.previous_sequence_id = Some(9);
        incr_1.sequence_id = Some(11);
        assert!(matches!(
            cache.update_order_book(incr_1),
            Err(PriceFeedError::SequenceValidation)
        ));

        Ok(())
    }
}
