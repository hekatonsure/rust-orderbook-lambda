use aws_config::BehaviorVersion;
use aws_sdk_s3::Client;
use chrono::{Datelike, Timelike, Utc};
use lambda_runtime::{Error, LambdaEvent, run, service_fn};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
struct BinanceRestDepth {
    bids: Vec<Vec<String>>,
    asks: Vec<Vec<String>>,
    #[serde(rename = "lastUpdateId")]
    last_update_id: i64,
}

async fn recovery_handler(event: LambdaEvent<Value>) -> Result<(), Error> {
    let s3 = Client::new(&aws_config::load_defaults(BehaviorVersion::latest()).await);
    let url = "https://api.binance.com/api/v3/depth?symbol=BTCUSDT&limit=1000";
    
    let resp = reqwest::get(url).await?.json::<BinanceRestDepth>().await?;
    let snapshot_time = resp.last_update_id;
    
    // Get last successful write from S3
    let last_key = get_last_key(&s3).await?;
    let gap_ms = snapshot_time - extract_timestamp(&last_key);
    
    if gap_ms > 5000 {
        println!("Backfilling gap of {}ms", gap_ms);
        // Write snapshot with REST data
        process_rest_snapshot(&resp, &s3).await?;
    }
    Ok(())
}

async fn get_last_key(s3: &Client) -> Result<String, Error> {
    let resp = s3.list_objects_v2()
        .bucket("orderbook-data")
        .prefix("orderbook/")
        .max_keys(1)
        .send()
        .await?;
    Ok(resp.contents().first().unwrap().key().unwrap().to_string())
}

async fn process_rest_snapshot(depth: &BinanceRestDepth, s3: &Client) -> Result<(), Error> {
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
    
    println!("Written recovery snapshot: {}", key);
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

fn extract_timestamp(key: &str) -> i64 {
    key.split('/').last().unwrap()
        .trim_end_matches(".avro")
        .parse().unwrap()
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(service_fn(recovery_handler)).await
}
