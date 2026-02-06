// src/main.rs

use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::error::Error;
use tokio_tungstenite::tungstenite::protocol::Message;
use url::Url;

use log::{debug, error, info, warn};
use serde_json::json;
use tokio::sync::mpsc;

pub mod settings;

#[derive(Debug, Deserialize)]
struct TradeEvent {
    e: String, // Event type
    E: u64,    // Event time
    s: String, // Symbol
    t: u64,    // Trade ID
    p: String, // Price
    q: String, // Quantity
    b: u64,    // Buyer order ID
    a: u64,    // Seller order ID
    T: u64,    // Trade time
    m: bool,   // Is the buyer the market maker?
    M: bool,   // Ignore
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum BinanceMessage {
    Event(TradeEvent),
    SubscriptionResponse {
        result: Option<serde_json::Value>,
        id: u64,
    },
    Other(serde_json::Value),
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logging
    settings::init_logging();
    info!("Starting Binance Futures Testnet WebSocket Client...");

    let url = "wss://fstream.binancefuture.com/ws";
    let url = Url::parse(url)?;

    let (mut ws_stream, _) = tokio_tungstenite::connect_async(url).await?;
    info!("WebSocket connection established.");

    // Send subscription message
    let subscribe_msg = json!({
        "method": "SUBSCRIBE",
        "params": ["ethusdt@trade"],
        "id": 1
    })
    .to_string();

    ws_stream.send(Message::Text(subscribe_msg)).await?;
    info!("Subscription message sent.");

    // Listen for incoming messages
    while let Some(message) = ws_stream.next().await {
        match message {
            Ok(Message::Text(text)) => {
                // Parse the message
                match serde_json::from_str::<BinanceMessage>(&text) {
                    Ok(parsed_msg) => match parsed_msg {
                        BinanceMessage::Event(trade) => {
                            info!(
                                "Trade - Symbol: {}, Price: {}, Quantity: {}, Buyer is Maker: {}",
                                trade.s, trade.p, trade.q, trade.m
                            );
                        }
                        BinanceMessage::SubscriptionResponse { result, id } => {
                            if id == 1 {
                                if result.is_none() {
                                    info!("Successfully subscribed to ethusdt@trade.");
                                } else {
                                    warn!("Subscription response with result: {:?}", result);
                                }
                            }
                        }
                        BinanceMessage::Other(other) => {
                            debug!("Other message: {:?}", other);
                        }
                    },
                    Err(e) => {
                        warn!("Failed to parse message: {}, error: {}", text, e);
                    }
                }
            }
            Ok(Message::Ping(ping)) => {
                info!("Received Ping, sending Pong.");
                ws_stream.send(Message::Pong(ping)).await?;
            }
            Ok(Message::Pong(_)) => {
                // Do nothing
            }
            Ok(Message::Close(frame)) => {
                info!("Received Close frame: {:?}", frame);
                break;
            }
            Err(e) => {
                error!("WebSocket error: {}", e);
                break;
            }
            _ => {
                warn!("Received unsupported message type.");
            }
        }
    }

    info!("WebSocket connection closed.");
    Ok(())
}
