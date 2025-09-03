use futures_util::StreamExt;
use serde_json::Value;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::connect_async;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing Binance.US WebSocket connection...");
    println!("Connecting to: wss://stream.binance.us:9443/ws/btcusdt@depth20@100ms");
    println!("{}", "=".repeat(60));

    let url = "wss://stream.binance.us:9443/ws/btcusdt@depth20@100ms";
    
    let (ws_stream, response) = connect_async(url).await?;
    println!("Connected successfully!");
    println!("Response: {:?}", response);
    println!("{}", "=".repeat(60));

    let (_, mut read) = ws_stream.split();
    
    let mut message_count = 0;
    let max_messages = 5;
    let timeout_duration = Duration::from_secs(10);

    println!("Receiving {} messages (timeout: {}s)...\n", max_messages, timeout_duration.as_secs());

    while message_count < max_messages {
        match timeout(timeout_duration, read.next()).await {
            Ok(Some(Ok(msg))) => {
                message_count += 1;
                
                if let Ok(text) = msg.to_text() {
                    // Check if it's a ping (just a number)
                    if text.chars().all(|c| c.is_digit(10)) {
                        println!("Message #{}: Ping (timestamp: {})", message_count, text);
                        println!();
                        continue;
                    }
                    
                    if let Ok(json) = serde_json::from_str::<Value>(text) {
                        println!("Message #{}", message_count);
                        println!("{}", "-".repeat(40));
                        
                        // For depth updates
                        if let Some(event_type) = json.get("e").and_then(|e| e.as_str()) {
                            println!("Event type: {}", event_type);
                            
                            if event_type == "depthUpdate" {
                                if let Some(symbol) = json.get("s") {
                                    println!("Symbol: {}", symbol);
                                }
                                if let Some(first_update) = json.get("U") {
                                    println!("First update ID: {}", first_update);
                                }
                                if let Some(last_update) = json.get("u") {
                                    println!("Last update ID: {}", last_update);
                                }
                                
                                if let Some(bids) = json.get("b").and_then(|b| b.as_array()) {
                                    println!("Bid updates: {} levels", bids.len());
                                    for (i, bid) in bids.iter().take(3).enumerate() {
                                        if let Some(bid_arr) = bid.as_array() {
                                            if bid_arr.len() >= 2 {
                                                println!("  Bid {}: {} @ {}", i+1, bid_arr[0], bid_arr[1]);
                                            }
                                        }
                                    }
                                }
                                
                                if let Some(asks) = json.get("a").and_then(|a| a.as_array()) {
                                    println!("Ask updates: {} levels", asks.len());
                                    for (i, ask) in asks.iter().take(3).enumerate() {
                                        if let Some(ask_arr) = ask.as_array() {
                                            if ask_arr.len() >= 2 {
                                                println!("  Ask {}: {} @ {}", i+1, ask_arr[0], ask_arr[1]);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        
                        // For partial book depth 
                        if let Some(last_update_id) = json.get("lastUpdateId") {
                            println!("Last Update ID: {}", last_update_id);
                            
                            if let Some(bids) = json.get("bids").and_then(|b| b.as_array()) {
                                println!("Bids: {} levels", bids.len());
                                if let Some(first_bid) = bids.first().and_then(|b| b.as_array()) {
                                    if first_bid.len() >= 2 {
                                        println!("  Best bid: {} @ {}", first_bid[0], first_bid[1]);
                                    }
                                }
                            }
                            
                            if let Some(asks) = json.get("asks").and_then(|a| a.as_array()) {
                                println!("Asks: {} levels", asks.len());
                                if let Some(first_ask) = asks.first().and_then(|a| a.as_array()) {
                                    if first_ask.len() >= 2 {
                                        println!("  Best ask: {} @ {}", first_ask[0], first_ask[1]);
                                    }
                                }
                            }
                        }
                        
                        println!();
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
    println!("Test completed successfully!");
    println!("Received {} messages", message_count);

    Ok(())
}