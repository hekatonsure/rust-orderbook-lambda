use aws_config::BehaviorVersion;
use aws_sdk_s3::Client;
use chrono::{Datelike, Timelike, Utc};
use lambda_runtime::{run, service_fn, Error, LambdaEvent};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct OrderBook {
    timestamp_ms: i64,
    bids: Vec<(f64, f64)>,
    asks: Vec<(f64, f64)>,
    spread: f64,
    mid_price: f64,
    imbalance_ratio: f64,
}

const SCHEMA: &str = r#"{"type":"record","name":"OrderBook","fields":[{"name":"timestamp_ms","type":"long"},{"name":"bids","type":{"type":"array","items":{"type":"array","items":"double"}}},{"name":"asks","type":{"type":"array","items":{"type":"array","items":"double"}}},{"name":"spread","type":"double"},{"name":"mid_price","type":"double"},{"name":"imbalance_ratio","type":"double"}]}"#;

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(service_fn(handler)).await
}

async fn handler(_: LambdaEvent<serde_json::Value>) -> Result<(), Error> {
    let s3 = Client::new(&aws_config::load_defaults(BehaviorVersion::latest()).await);
    
    // Check gap from last write
    let objs = s3.list_objects_v2()
        .bucket("orderbook-data")
        .prefix("orderbook/")
        .send()
        .await?;
    
    let now = Utc::now().timestamp_millis();
    let last_ts = objs.contents()
        .last()  // actually get the LAST one
        .and_then(|obj| obj.key()?.split('/').last()?.strip_suffix(".avro"))
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    
    if now - last_ts > 5000 {  // 5 second gap
        println!("Backfilling {}ms gap", now - last_ts);
        
        // Fetch REST snapshot
        let depth: serde_json::Value = reqwest::get("https://api.binance.com/api/v3/depth?symbol=BTCUSDT&limit=1000")
            .await?
            .json()
            .await?;
        
        // Parse books
        let parse = |key| -> Vec<(f64, f64)> {
            depth[key].as_array().unwrap().iter().take(20)
                .map(|x| (x[0].as_str().unwrap().parse().unwrap(),
                         x[1].as_str().unwrap().parse().unwrap()))
                .collect()
        };
        let (bids, asks) = (parse("bids"), parse("asks"));
        
        // Metrics
        let mid = (bids[0].0 + asks[0].0) / 2.0;
        let vol = |b: &[(f64, f64)]| -> f64 { b[..5].iter().map(|x| x.1).sum() };
        let (bv, av) = (vol(&bids), vol(&asks));
        
        // Normalize
        let norm = |book: &[(f64, f64)], is_ask| -> Vec<(f64, f64)> {
            [0.0001, 0.0005, 0.001, 0.005, 0.01].iter().map(|&d| {
                let target = mid * (1.0 + if is_ask { d } else { -d });
                (target, book.iter()
                    .filter(|(p, _)| (is_ask && *p <= target) || (!is_ask && *p >= target))
                    .map(|(_, q)| q)
                    .sum())
            }).collect()
        };
        
        // Build & serialize
        let book = OrderBook {
            timestamp_ms: now,
            bids: norm(&bids, false),
            asks: norm(&asks, true),
            spread: asks[0].0 - bids[0].0,
            mid_price: mid,
            imbalance_ratio: (bv - av) / (bv + av),
        };
        
        let schema = apache_avro::Schema::parse_str(SCHEMA)?;
        let mut writer = apache_avro::Writer::new(&schema, Vec::new());
        writer.append_ser(&book)?;
        
        let t = Utc::now();
        let key = format!("orderbook/year={}/month={:02}/day={:02}/hour={:02}/{}.avro",
                         t.year(), t.month(), t.day(), t.hour(), now);
        
        s3.put_object()
            .bucket("orderbook-data")
            .key(&key)
            .body(writer.into_inner()?.into())
            .send()
            .await?;
        
        println!("Recovered: {}", key);
    }
    Ok(())
}