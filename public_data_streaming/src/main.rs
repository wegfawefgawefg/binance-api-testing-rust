use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use std::error::Error;
use std::time::{Duration, Instant};
use tokio::time::interval;
use tokio_tungstenite::tungstenite::protocol::Message;
use url::Url;

#[allow(unused_imports)]
use log::{debug, error, info, warn};
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;

pub mod models;
pub mod settings;

/// Binance public WebSocket endpoints for testnet and mainnet
const TESTNET_WS_BASE_URL: &str = "wss://testnet.binance.vision/ws";
const MAINNET_WS_BASE_URL: &str = "wss://stream.binance.com:9443/ws";
const STATS_INTERVAL_SECS: u64 = 5;
const UNSOLICITED_PONG_INTERVAL_SECS: u64 = 180;

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

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

        let (ws_stream, _) = tokio_tungstenite::connect_async(url).await?;
        info!("Connected to WebSocket: {}", self.ws_url);
        info!("Subscribed via stream URL: {}", stream);

        let (write, read) = ws_stream.split();
        self.run_websocket_loop(write, read).await?;

        Ok(())
    }

    async fn run_websocket_loop(
        &self,
        mut write: SplitSink<WsStream, Message>,
        mut read: SplitStream<WsStream>,
    ) -> Result<(), Box<dyn Error>> {
        let start_time = Instant::now();
        let mut message_count = 0usize;
        let mut last_message_time = Instant::now();
        let mut print_stats_interval = interval(Duration::from_secs(STATS_INTERVAL_SECS));
        let mut pong_interval = interval(Duration::from_secs(UNSOLICITED_PONG_INTERVAL_SECS));

        loop {
            tokio::select! {
                msg = read.next() => {
                    if !self.handle_message(msg, &mut write, &mut message_count, &mut last_message_time).await? {
                        break;
                    }
                }
                _ = print_stats_interval.tick() => {
                    self.print_stats(start_time, message_count);
                }
                _ = pong_interval.tick() => {
                    self.send_unsolicited_pong(&mut write).await?;
                }
            }
        }

        Ok(())
    }

    async fn handle_message(
        &self,
        msg: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
        write: &mut SplitSink<WsStream, Message>,
        message_count: &mut usize,
        last_message_time: &mut Instant,
    ) -> Result<bool, Box<dyn Error>> {
        match msg {
            Some(Ok(Message::Text(text))) => {
                self.handle_text_message(&text, message_count, last_message_time);
                Ok(true)
            }
            Some(Ok(Message::Ping(payload))) => {
                info!("Received Ping, sending Pong.");
                write.send(Message::Pong(payload)).await?;
                Ok(true)
            }
            Some(Ok(Message::Pong(_))) => Ok(true),
            Some(Ok(Message::Close(frame))) => {
                if let Some(cf) = frame {
                    info!("WebSocket closed: {:?}", cf);
                } else {
                    info!("WebSocket closed without a close frame.");
                }
                Ok(false)
            }
            Some(Err(e)) => {
                error!("WebSocket error: {}", e);
                Ok(false)
            }
            None => {
                warn!("WebSocket stream ended.");
                Ok(false)
            }
            _ => Ok(true),
        }
    }

    fn handle_text_message(&self, message: &str, message_count: &mut usize, last_message_time: &mut Instant) {
        let now = Instant::now();
        let time_since_last = now.duration_since(*last_message_time);
        *last_message_time = now;
        *message_count += 1;
        debug!("Time since last message: {:?}", time_since_last);

        match serde_json::from_str::<models::BinanceMessage>(message) {
            Ok(models::BinanceMessage::Event(models::BinanceEvent::Trade(trade))) => {
                info!(
                    "Trade - Symbol: {}, Price: {}, Quantity: {}, Trade Time: {}",
                    trade.symbol, trade.price, trade.quantity, trade.trade_time
                );
            }
            Ok(models::BinanceMessage::SubscriptionResponse { result, id }) => {
                info!("Subscription response: result={:?}, id={}", result, id);
            }
            Ok(models::BinanceMessage::Other(other)) => {
                debug!("Other message: {:?}", other);
            }
            Ok(other) => {
                debug!("Non-trade event: {:?}", other);
            }
            Err(e) => {
                warn!("Failed to deserialize message: {}, error: {}", message, e);
            }
        }
    }

    fn print_stats(&self, start_time: Instant, message_count: usize) {
        let elapsed = start_time.elapsed().as_secs_f64();
        info!(
            "Messages received: {}, Frequency: {:.2} msg/s",
            message_count,
            message_count as f64 / elapsed
        );
    }

    async fn send_unsolicited_pong(
        &self,
        write: &mut SplitSink<WsStream, Message>,
    ) -> Result<(), Box<dyn Error>> {
        debug!("Sending unsolicited pong heartbeat.");
        write.send(Message::Pong(vec![])).await?;
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
