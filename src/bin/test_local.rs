use chrono::{Datelike, Timelike, Utc};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::connect_async;

#[derive(Serialize, Deserialize, Debug)]
struct OrderBook {
    timestamp_ms: i64,
    bids: Vec<(f64, f64)>,
    asks: Vec<(f64, f64)>,
    spread: f64,
    mid_price: f64,
    imbalance_ratio: f64,
}

#[derive(Deserialize, Debug)]
struct BinanceDepth {
    #[serde(rename = "lastUpdateId")]
    last_update_id: i64,
    bids: Vec<Vec<String>>,
    asks: Vec<Vec<String>>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing Orderbook Lambda locally (writing to ./data instead of S3)");
    println!("{}", "=".repeat(60));
    
    // Create local data directory
    fs::create_dir_all("./data")?;
    
    let url = "wss://stream.binance.us:9443/ws/btcusdt@depth20@100ms";
    println!("Connecting to: {}", url);
    
    let (ws_stream, response) = connect_async(url).await?;
    println!("Connected! Response: {:?}", response);
    println!("{}", "=".repeat(60));
    
    let (_, mut read) = ws_stream.split();
    
    let mut message_count = 0;
    let max_messages = 10;
    let timeout_duration = Duration::from_secs(30);
    
    println!("Processing {} messages (timeout: {}s)...\n", max_messages, timeout_duration.as_secs());
    
    while message_count < max_messages {
        match timeout(timeout_duration, read.next()).await {
            Ok(Some(Ok(msg))) => {
                if let Ok(text) = msg.to_text() {
                    // Skip ping messages
                    if text.chars().all(|c| c.is_digit(10)) {
                        continue;
                    }
                    
                    match serde_json::from_str::<BinanceDepth>(text) {
                        Ok(depth) => {
                            message_count += 1;
                            
                            let bids: Vec<(f64, f64)> = depth.bids.iter()
                                .take(20)
                                .filter_map(|b| {
                                    if b.len() >= 2 {
                                        Some((b[0].parse().ok()?, b[1].parse().ok()?))
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            
                            let asks: Vec<(f64, f64)> = depth.asks.iter()
                                .take(20)
                                .filter_map(|a| {
                                    if a.len() >= 2 {
                                        Some((a[0].parse().ok()?, a[1].parse().ok()?))
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            
                            if bids.is_empty() || asks.is_empty() {
                                println!("Skipping message with empty bids or asks");
                                continue;
                            }
                            
                            let mid_price = (bids[0].0 + asks[0].0) / 2.0;
                            let spread = asks[0].0 - bids[0].0;
                            
                            let bid_vol: f64 = bids.iter().take(5).map(|b| b.1).sum();
                            let ask_vol: f64 = asks.iter().take(5).map(|a| a.1).sum();
                            let imbalance_ratio = if bid_vol + ask_vol > 0.0 {
                                (bid_vol - ask_vol) / (bid_vol + ask_vol)
                            } else {
                                0.0
                            };
                            
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
                            
                            // Write to local file instead of S3
                            let now = Utc::now();
                            let dir = format!(
                                "./data/orderbook/year={}/month={:02}/day={:02}/hour={:02}",
                                now.year(), now.month(), now.day(), now.hour()
                            );
                            fs::create_dir_all(&dir)?;
                            
                            let filename = format!("{}/{}.json", dir, now.timestamp_millis());
                            let json = serde_json::to_string_pretty(&book)?;
                            fs::write(&filename, json)?;
                            
                            println!("Message #{}: Written to {}", message_count, filename);
                            println!("  Mid price: ${:.2}", mid_price);
                            println!("  Spread: ${:.2}", spread);
                            println!("  Imbalance ratio: {:.4}", imbalance_ratio);
                            println!("  Best bid: ${:.2} @ {:.5} BTC", bids[0].0, bids[0].1);
                            println!("  Best ask: ${:.2} @ {:.5} BTC", asks[0].0, asks[0].1);
                            println!();
                        }
                        Err(e) => {
                            eprintln!("Failed to parse depth message: {}", e);
                            eprintln!("Raw message: {}", text);
                        }
                    }
                }
            }
            Ok(Some(Err(e))) => {
                eprintln!("WebSocket error: {}", e);
                break;
            }
            Ok(None) => {
                println!("WebSocket stream ended");
                break;
            }
            Err(_) => {
                println!("Timeout reached");
                break;
            }
        }
    }
    
    println!("{}", "=".repeat(60));
    println!("Test completed! Processed {} messages", message_count);
    
    // Show files created
    println!("\nFiles created:");
    for entry in fs::read_dir("./data")? {
        if let Ok(entry) = entry {
            println!("  {}", entry.path().display());
        }
    }
    
    Ok(())
}

fn normalize_to_depths(levels: &[(f64, f64)], mid: f64, is_ask: bool) -> Vec<(f64, f64)> {
    let depths = vec![0.0001, 0.0005, 0.001, 0.005, 0.01];
    depths.iter().map(|&d| {
        let target_price = if is_ask {
            mid * (1.0 + d)
        } else {
            mid * (1.0 - d)
        };
        
        let mut cumulative_volume = 0.0;
        for &(price, volume) in levels {
            if (is_ask && price <= target_price) || (!is_ask && price >= target_price) {
                cumulative_volume += volume;
            }
        }
        
        (target_price, cumulative_volume)
    }).collect()
}