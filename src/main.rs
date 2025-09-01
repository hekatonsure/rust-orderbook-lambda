use aws_sdk_s3::Client;
use chrono::Utc;
use futures_util::StreamExt;
use lambda_runtime::{run, service_fn, Error, LambdaEvent};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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

#[derive(Deserialize)]
struct BinanceDepth {
    bids: Vec<Vec<String>>,
    asks: Vec<Vec<String>>,
}

async fn function_handler(_: LambdaEvent<serde_json::Value>) -> Result<(), Error> {
    let s3 = Client::new(&aws_config::load_from_env().await);
    let url = "wss://stream.binance.com:9443/ws/btcusdt@depth20@100ms";
    
    let (ws_stream, _) = connect_async(url).await?;
    let (_, mut read) = ws_stream.split();
    
    let mut backoff = 1;
    while let Some(msg) = read.next().await {
        match process_message(msg?, &s3).await {
            Ok(_) => backoff = 1,
            Err(e) => {
                eprintln!("Error: {}", e);
                tokio::time::sleep(tokio::time::Duration::from_secs(backoff)).await;
                backoff = (backoff * 2).min(30);
            }
        }
    }
    Ok(())
}

async fn process_message(msg: tungstenite::Message, s3: &Client) -> Result<(), Error> {
    let depth: BinanceDepth = serde_json::from_str(&msg.to_string())?;
    
    let bids: Vec<(f64, f64)> = depth.bids.iter()
        .take(20)
        .map(|b| (b[0].parse().unwrap(), b[1].parse().unwrap()))
        .collect();
    
    let asks: Vec<(f64, f64)> = depth.asks.iter()
        .take(20)
        .map(|a| (a[0].parse().unwrap(), a[1].parse().unwrap()))
        .collect();
    
    let mid_price = (bids[0].0 + asks[0].0) / 2.0;
    let spread = asks[0].0 - bids[0].0;
    
    let bid_vol: f64 = bids.iter().take(5).map(|b| b.1).sum();
    let ask_vol: f64 = asks.iter().take(5).map(|a| a.1).sum();
    let imbalance_ratio = (bid_vol - ask_vol) / (bid_vol + ask_vol);
    
    let normalized_bids = normalize_to_depths(&bids, mid_price, false);
    let normalized_asks = normalize_to_depths(&asks, mid_price, true);
    
    let book = OrderBook {
        timestamp_ms: Utc::now().timestamp_millis(),
        bids: normalized_bids,
        asks: normalized_asks,
        spread,
        mid_price,
        imbalance_ratio,
    };
    
    let avro_bytes = serialize_avro(&book)?;
    let now = Utc::now();
    let key = format!(
        "orderbook/year={}/month={:02}/day={:02}/hour={:02}/{}.avro",
        now.year(), now.month(), now.day(), now.hour(), now.timestamp_millis()
    );
    
    s3.put_object()
        .bucket("orderbook-data")
        .key(&key)
        .body(avro_bytes.into())
        .send()
        .await?;
    
    println!("Written: {}", key);
    Ok(())
}

fn normalize_to_depths(levels: &[(f64, f64)], mid: f64, is_ask: bool) -> Vec<(f64, f64)> {
    let depths = vec![0.0001, 0.0005, 0.001, 0.005, 0.01];
    depths.iter().map(|&d| {
        let target = if is_ask { mid * (1.0 + d) } else { mid * (1.0 - d) };
        let mut cum_qty = 0.0;
        for (price, qty) in levels {
            if (is_ask && *price <= target) || (!is_ask && *price >= target) {
                cum_qty += qty;
            }
        }
        (target, cum_qty)
    }).collect()
}

fn serialize_avro(book: &OrderBook) -> Result<Vec<u8>, Error> {
    let schema = apache_avro::Schema::parse_str(SCHEMA)?;
    let mut writer = apache_avro::Writer::new(&schema, Vec::new());
    writer.append_ser(book)?;
    Ok(writer.into_inner()?)
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
    run(service_fn(function_handler)).await
}
