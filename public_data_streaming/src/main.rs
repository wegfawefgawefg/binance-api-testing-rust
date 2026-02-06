// src/main.rs

use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::error::Error;
use tokio_tungstenite::tungstenite::protocol::Message;
use url::Url;

#[allow(unused_imports)]
use log::{debug, error, info, warn};

pub mod settings;

/// Binance public WebSocket endpoints for testnet and mainnet
const TESTNET_WS_BASE_URL: &str = "wss://testnet.binance.vision/ws";
const MAINNET_WS_BASE_URL: &str = "wss://stream.binance.com:9443/ws";

#[allow(dead_code, non_snake_case)]
#[derive(Debug, Deserialize)]
struct TradeEvent {
    e: String, // Event type
    E: u64,    // Event time
    s: String, // Symbol
    t: u64,    // Trade ID
    p: String, // Price
    q: String, // Quantity
    #[serde(default)]
    b: Option<u64>, // Buyer order ID
    #[serde(default)]
    a: Option<u64>, // Seller order ID
    T: u64,    // Trade time
    m: bool,   // Is the buyer the market maker?
    M: bool,   // Ignore
}

struct BinancePublicWebSocket {
    ws_url: String,
    symbol: String,
}

impl BinancePublicWebSocket {
    /// Creates a new instance for a specific symbol.
    /// `symbol` should be in lowercase, e.g., "ethusdt"
    fn new(symbol: &str, use_testnet: bool) -> Self {
        let ws_url = if use_testnet {
            TESTNET_WS_BASE_URL.to_string()
        } else {
            MAINNET_WS_BASE_URL.to_string()
        };
        Self {
            ws_url,
            symbol: symbol.to_lowercase(),
        }
    }

    /// Constructs the stream name based on the symbol.
    fn stream_name(&self) -> String {
        format!("{}@trade", self.symbol)
    }

    /// Connects to the WebSocket and listens for trade events.
    async fn connect_and_listen(&self) -> Result<(), Box<dyn Error>> {
        let stream = self.stream_name();
        let url = format!("{}/{}", self.ws_url, stream);
        let url = Url::parse(&url)?;

        let (mut ws_stream, _) = tokio_tungstenite::connect_async(url).await?;
        info!("Connected to WebSocket: {}", self.ws_url);

        while let Some(message) = ws_stream.next().await {
            match message {
                Ok(Message::Text(text)) => {
                    self.handle_message(&text)?;
                }
                Ok(Message::Close(frame)) => {
                    if let Some(cf) = frame {
                        info!("WebSocket closed: {:?}", cf);
                    } else {
                        info!("WebSocket closed without a close frame.");
                    }
                    break;
                }
                Ok(Message::Ping(payload)) => {
                    info!("Received Ping, sending Pong.");
                    ws_stream.send(Message::Pong(payload)).await?;
                }
                Ok(Message::Pong(_)) => {
                    // Pong received; no action needed
                }
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    break;
                }
                _ => {
                    warn!("Received unsupported message: {:?}", message);
                }
            }
        }

        Ok(())
    }

    fn handle_message(&self, message: &str) -> Result<(), Box<dyn Error>> {
        let trade: TradeEvent = match serde_json::from_str(message) {
            Ok(trade) => trade,
            Err(e) => {
                warn!("Failed to deserialize message: {}, error: {}", message, e);
                return Ok(());
            }
        };

        // Log trade details, handling optional fields
        info!(
            "Trade - ID: {}, Price: {}, Quantity: {}, Buyer is Maker: {}, Buyer Order ID: {:?}, Seller Order ID: {:?}",
            trade.t, trade.p, trade.q, trade.m, trade.b, trade.a
        );

        // Additional processing can be done here

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logging
    settings::init_logging();

    info!("Starting Binance Public WebSocket Client...");

    // Specify the symbol you want to subscribe to, e.g., "ethusdt"
    let symbol = "ethusdt";

    // Set to `true` to use the Testnet; set to `false` for Mainnet
    let use_testnet = false;

    let ws_client = BinancePublicWebSocket::new(symbol, use_testnet);

    // Connect and listen for trade events
    if let Err(e) = ws_client.connect_and_listen().await {
        error!("Error in WebSocket connection: {}", e);
    }

    Ok(())
}
