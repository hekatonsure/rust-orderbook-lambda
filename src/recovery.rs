use aws_sdk_s3::Client;
use lambda_runtime::{run, service_fn, Error, LambdaEvent};
use serde_json::Value;

async fn recovery_handler(event: LambdaEvent<Value>) -> Result<(), Error> {
    let s3 = Client::new(&aws_config::load_from_env().await);
    let url = "https://api.binance.com/api/v3/depth?symbol=BTCUSDT&limit=1000";
    
    let resp = reqwest::get(url).await?.json::<Value>().await?;
    let snapshot_time = resp["lastUpdateId"].as_i64().unwrap();
    
    // Get last successful write from S3
    let last_key = get_last_key(&s3).await?;
    let gap_ms = snapshot_time - extract_timestamp(&last_key);
    
    if gap_ms > 5000 {
        println!("Backfilling gap of {}ms", gap_ms);
        // Write snapshot with REST data
        process_message(serde_json::to_string(&resp)?.into(), &s3).await?;
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

fn extract_timestamp(key: &str) -> i64 {
    key.split('/').last().unwrap()
        .trim_end_matches(".avro")
        .parse().unwrap()
}
