// src/main.rs

use futures::{SinkExt, StreamExt}; // For StreamExt and SinkExt traits
use std::env;
use std::error::Error;
use std::time::Duration;
use tokio::time;
use tokio_tungstenite::tungstenite::protocol::Message;
use url::Url;

mod settings;
use dotenv::dotenv;

#[allow(unused_imports)]
use log::{debug, error, info, warn};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;

// =============================== Configuration ===============================

const TESTNET_API_BASE_URL: &str = "https://testnet.binancefuture.com";
const TESTNET_WS_BASE_URL: &str = "wss://fstream.binancefuture.com/ws"; // WebSocket base URL

// =============================== Data Structures ===============================

#[derive(Debug, Deserialize)]
struct ListenKeyResponse {
    #[serde(rename = "listenKey")]
    listen_key: String,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    code: i32,
    msg: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "e")]
enum BinanceEvent {
    #[serde(rename = "ORDER_TRADE_UPDATE")]
    OrderTradeUpdate(OrderTradeUpdate),

    #[serde(rename = "TRADE_LITE")]
    TradeLite(TradeLite),

    #[serde(rename = "ACCOUNT_UPDATE")]
    AccountUpdate(AccountUpdate),
    // Add other event types here as needed
}

#[derive(Debug, Deserialize)]
struct OrderTradeUpdate {
    #[serde(rename = "o")]
    order_detail: OrderDetail,
}

#[derive(Debug, Deserialize)]
struct OrderDetail {
    #[serde(rename = "i")]
    order_id: u64, // Changed from String to u64 based on the error message
    #[serde(rename = "X")]
    order_status: String, // Order status
}

#[derive(Debug, Deserialize)]
struct TradeLite {
    #[serde(rename = "i")]
    trade_id: u64,
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "q")]
    quantity: String,
    #[serde(rename = "p")]
    price: String,
    #[serde(rename = "m")]
    is_maker: bool,
    // Add other fields as necessary
}

#[derive(Debug, Deserialize)]
struct AccountUpdate {
    #[serde(rename = "a")]
    account_info: AccountInfo,
}

#[derive(Debug, Deserialize)]
struct AccountInfo {
    #[serde(rename = "B")]
    balances: Vec<Balance>,
    #[serde(rename = "P")]
    positions: Vec<Position>,
    #[serde(rename = "m")]
    event_type: String,
}

#[derive(Debug, Deserialize)]
struct Balance {
    #[serde(rename = "a")]
    asset: String,
    #[serde(rename = "wb")]
    available_balance: String,
    #[serde(rename = "cw")]
    cross_wallet_balance: String,
    #[serde(rename = "bc")]
    balance_change: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct Position {
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "pa")]
    position_amount: String,
    #[serde(rename = "ep")]
    entry_price: String,
    #[serde(rename = "cr")]
    accumulated_realized: String,
    #[serde(rename = "up")]
    unrealized_profit: String,
    #[serde(rename = "mt")]
    margin_type: String,
    #[serde(rename = "iw")]
    isolated_wallet: String,
    #[serde(rename = "ps")]
    position_side: String,
    #[serde(rename = "ma")]
    margin_asset: String,
    #[serde(rename = "bep")]
    break_even_price: String,
}

// =============================== Helper Functions ===============================

async fn create_listen_key() -> Result<String, Box<dyn Error>> {
    let api_key = get_api_key()?;
    let url = format!("{}/fapi/v1/listenKey", TESTNET_API_BASE_URL);
    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    headers.insert("X-MBX-APIKEY", HeaderValue::from_str(&api_key)?);

    let resp = client.post(&url).headers(headers).send().await?;
    let status = resp.status(); // Extract status before consuming resp

    if status.is_success() {
        let data: ListenKeyResponse = resp.json().await?;
        info!("Listen Key: {}", data.listen_key);
        Ok(data.listen_key)
    } else {
        let error_text = resp.text().await?;
        match serde_json::from_str::<ErrorResponse>(&error_text) {
            Ok(err) => {
                error!("Error {}: {}", err.code, err.msg);
                Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    err.msg,
                )))
            }
            Err(_) => {
                error!("HTTP Error {}: {}", status, error_text);
                Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Failed to parse error response",
                )))
            }
        }
    }
}

async fn renew_listen_key(listen_key: &str) -> Result<(), Box<dyn Error>> {
    let api_key = get_api_key()?;
    let url = format!("{}/fapi/v1/listenKey", TESTNET_API_BASE_URL);
    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    headers.insert("X-MBX-APIKEY", HeaderValue::from_str(&api_key)?);
    headers.insert(
        "Content-Type",
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );

    let params = [("listenKey", listen_key)];

    let resp = client
        .put(&url)
        .headers(headers)
        .form(&params)
        .send()
        .await?;
    let status = resp.status(); // Extract status before consuming resp

    if status.is_success() {
        info!("Listen key renewed successfully.");
        Ok(())
    } else {
        let error_text = resp.text().await?;
        match serde_json::from_str::<ErrorResponse>(&error_text) {
            Ok(err) => {
                error!("Error {}: {}", err.code, err.msg);
                Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    err.msg,
                )))
            }
            Err(_) => {
                error!("HTTP Error {}: {}", status, error_text);
                Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Failed to parse error response",
                )))
            }
        }
    }
}

// =============================== WebSocket Client ===============================

struct BinanceWebSocketClient {
    ws_url: String,
}

impl BinanceWebSocketClient {
    fn new(listen_key: String) -> Self {
        let ws_url = format!("{}/{}", TESTNET_WS_BASE_URL, listen_key);
        Self { ws_url }
    }

    async fn connect_and_listen(&self) -> Result<(), Box<dyn Error>> {
        let url = Url::parse(&self.ws_url)?;
        let (mut ws_stream, _) = tokio_tungstenite::connect_async(url).await?;
        info!("WebSocket handshake successful.");

        while let Some(message) = ws_stream.next().await {
            match message {
                Ok(Message::Text(text)) => {
                    self.handle_message(&text).await?;
                }
                Ok(Message::Close(_)) => {
                    info!("Received close frame from server.");
                    break;
                }
                Ok(Message::Ping(payload)) => {
                    info!("Received ping, sending pong.");
                    ws_stream.send(Message::Pong(payload)).await?;
                }
                Ok(Message::Pong(_)) => {
                    // Do nothing
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

    async fn handle_message(&self, message: &str) -> Result<(), Box<dyn Error>> {
        // Deserialize the message into BinanceEvent enum
        let event: BinanceEvent = match serde_json::from_str(message) {
            Ok(ev) => ev,
            Err(e) => {
                warn!("Failed to deserialize message: {}, error: {}", message, e);
                return Ok(());
            }
        };

        match event {
            BinanceEvent::OrderTradeUpdate(update) => {
                let order_id = update.order_detail.order_id;
                let status = update.order_detail.order_status;
                info!("Order Update - ID: {}, Status: {}", order_id, status);
                if ["FILLED", "CANCELED", "REJECTED", "EXPIRED"].contains(&status.as_str()) {
                    info!("✅ Order {} has been {}.", order_id, status.to_lowercase());
                    // Optionally, close the WebSocket connection here if desired
                }
            }
            BinanceEvent::TradeLite(trade) => {
                info!(
                    "Trade Lite - Trade ID: {}, Symbol: {}, Quantity: {}, Price: {}, Maker: {}",
                    trade.trade_id, trade.symbol, trade.quantity, trade.price, trade.is_maker
                );
                // Add additional processing logic as needed
            }
            BinanceEvent::AccountUpdate(account_update) => {
                info!(
                    "Account Update - Event Type: {}",
                    account_update.account_info.event_type
                );
                for balance in account_update.account_info.balances {
                    info!(
                        "Balance - Asset: {}, Available: {}, Cross Wallet: {}, Balance Change: {}",
                        balance.asset,
                        balance.available_balance,
                        balance.cross_wallet_balance,
                        balance.balance_change
                    );
                }
                for position in account_update.account_info.positions {
                    info!(
                        "Position - Symbol: {}, Amount: {}, Entry Price: {}, Unrealized Profit: {}",
                        position.symbol,
                        position.position_amount,
                        position.entry_price,
                        position.unrealized_profit
                    );
                }
                // Add additional processing logic as needed
            } // Handle other event types if necessary
        }

        Ok(())
    }
}

// =============================== Main Execution ===============================

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenv().ok();
    settings::init_logging();

    info!("Starting Binance WebSocket client...");

    // Step 1: Create a listen key
    let listen_key = match create_listen_key().await {
        Ok(key) => key,
        Err(e) => {
            error!("Failed to create listen key: {}", e);
            return Ok(());
        }
    };

    // Step 2: Initialize WebSocket client
    let ws_client = BinanceWebSocketClient::new(listen_key.clone());

    // Step 3: Start a task to handle WebSocket connection
    let ws_handle = tokio::spawn(async move {
        if let Err(e) = ws_client.connect_and_listen().await {
            error!("WebSocket error: {}", e);
        }
    });

    // Step 4: Start a task to renew the listen key every 55 minutes
    let listen_key_clone = listen_key.clone();
    let renew_handle = tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(55 * 60));
        loop {
            interval.tick().await;
            match renew_listen_key(&listen_key_clone).await {
                Ok(_) => {}
                Err(e) => {
                    error!("Failed to renew listen key: {}", e);
                    break;
                }
            }
        }
    });

    // Optional: Handle graceful shutdown on Ctrl+C
    let shutdown_handle = tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to listen for Ctrl+C");
        info!("Received Ctrl+C, shutting down.");
    });

    // Wait for any of the tasks to complete
    tokio::select! {
        _ = ws_handle => {
            info!("WebSocket connection closed.");
        },
        _ = renew_handle => {
            info!("Listen key renewal task ended.");
        },
        _ = shutdown_handle => {
            info!("Shutdown signal received.");
        },
    }

    // Optionally, perform cleanup here

    Ok(())
}

fn get_api_key() -> Result<String, Box<dyn Error>> {
    env::var("BINANCE_API_KEY")
        .map_err(|_| "Missing BINANCE_API_KEY environment variable".into())
}
