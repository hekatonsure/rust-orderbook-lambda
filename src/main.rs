use aws_config::BehaviorVersion;
use aws_sdk_s3::Client;
use chrono::{Datelike, Timelike, Utc};
use futures_util::StreamExt;
use lambda_runtime::{run, service_fn, Error, LambdaEvent};
use serde::{Deserialize, Serialize};
use tokio_tungstenite::connect_async;

#[derive(Serialize, Deserialize)]
struct OrderBook {
    timestamp_ms: i64,
    bids: Vec<(f64, f64)>,
    asks: Vec<(f64, f64)>,
    spread: f64,
    mid_price: f64,
    imbalance_ratio: f64,
}

const SCHEMA: &str = r#"
{
  "type": "record",
  "name": "OrderBook",
  "fields": [
    {"name": "timestamp_ms", "type": "long"},
    {"name": "bids", "type": {"type": "array", "items": {"type": "array", "items": "double"}}},
    {"name": "asks", "type": {"type": "array", "items": {"type": "array", "items": "double"}}},
    {"name": "spread", "type": "double"},
    {"name": "mid_price", "type": "double"},
    {"name": "imbalance_ratio", "type": "double"}
  ]
}
"#;

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(service_fn(handler)).await
}

async fn handler(_: LambdaEvent<serde_json::Value>) -> Result<(), Error> {
    let s3 = Client::new(&aws_config::load_defaults(BehaviorVersion::latest()).await);
    let (ws, _) = connect_async("wss://stream.binance.us:9443/ws/btcusdt@depth20@100ms").await?;
    let (_, mut rx) = ws.split();
    
    while let Some(msg) = rx.next().await {
        let txt = msg?.to_text()?.to_string();  // handles all message types
        let v: serde_json::Value = serde_json::from_str(&txt)?;
        
        // parse books
        let parse_book = |key| -> Vec<(f64, f64)> {
            v[key].as_array().unwrap().iter().take(20)
                .map(|x| (x[0].as_str().unwrap().parse().unwrap(), 
                         x[1].as_str().unwrap().parse().unwrap()))
                .collect()
        };
        let (bids, asks) = (parse_book("bids"), parse_book("asks"));
        
        // core metrics
        let mid = (bids[0].0 + asks[0].0) / 2.0;
        let spread = asks[0].0 - bids[0].0;
        let vol = |book: &[(f64, f64)]| -> f64 { book[..5].iter().map(|x| x.1).sum() };
        let (bid_vol, ask_vol) = (vol(&bids), vol(&asks));
        
        // normalize to depth levels
        let depths = [0.0001, 0.0005, 0.001, 0.005, 0.01];
        let norm = |book: &[(f64, f64)], is_ask: bool| -> Vec<(f64, f64)> {
            depths.iter().map(|&d| {
                let target = mid * (1.0 + if is_ask { d } else { -d });
                let cum: f64 = book.iter()
                    .filter(|(p, _)| (is_ask && *p <= target) || (!is_ask && *p >= target))
                    .map(|(_, q)| q)
                    .sum();
                (target, cum)
            }).collect()
        };
        
        let book = OrderBook {
            timestamp_ms: Utc::now().timestamp_millis(),
            bids: norm(&bids, false),
            asks: norm(&asks, true),
            spread,
            mid_price: mid,
            imbalance_ratio: (bid_vol - ask_vol) / (bid_vol + ask_vol),
        };

        let schema = apache_avro::Schema::parse_str(SCHEMA)?;
        let mut writer = apache_avro::Writer::new(&schema, Vec::new());
        writer.append_ser(&book)?;
        
        let now = Utc::now();
        let key = format!("orderbook/year={}/month={:02}/day={:02}/hour={:02}/{}.avro",
                         now.year(), now.month(), now.day(), now.hour(), now.timestamp_millis());
        
        s3.put_object()
            .bucket("orderbook-data")
            .key(&key)
            .body(writer.into_inner()?.into())
            .send()
            .await?;
        
        println!("Written: {}", key);
    }
    Ok(())
}