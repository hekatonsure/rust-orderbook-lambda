use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures_util::StreamExt;

#[tokio::main]
async fn main() {
    println!("Testing Binance WebSocket connection...");
    
    // Using Binance US endpoint - symbol is "btcusd" not "btcusdt"
    let url = "wss://stream.binance.us:9443/ws/btcusd@depth20@100ms";
    
    match connect_async(url).await {
        Ok((ws_stream, _)) => {
            println!("✓ Connected to Binance WebSocket");
            let (_, mut read) = ws_stream.split();
            
            let mut count = 0;
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                            println!("\n✓ Received orderbook update #{}", count + 1);
                            println!("  Event type: {}", json["e"].as_str().unwrap_or("unknown"));
                            println!("  Symbol: {}", json["s"].as_str().unwrap_or("unknown"));
                            
                            if let Some(bids) = json["bids"].as_array() {
                                println!("  Bids: {} levels", bids.len());
                            }
                            if let Some(asks) = json["asks"].as_array() {
                                println!("  Asks: {} levels", asks.len());
                            }
                        }
                        
                        count += 1;
                        if count >= 3 {
                            println!("\n✓ Successfully received 3 updates. Connection working!");
                            break;
                        }
                    }
                    Ok(Message::Close(_)) => {
                        println!("Connection closed");
                        break;
                    }
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
        }
        Err(e) => {
            eprintln!("✗ Failed to connect: {}", e);
            eprintln!("  Check your internet connection and firewall settings");
        }
    }
}