use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use public_data_streaming::models;
use public_data_streaming::settings;
use std::env;
use std::error::Error;
use std::time::{Duration, Instant};
use tokio::time::interval;
use tokio_tungstenite::tungstenite::protocol::Message;
use url::Url;

#[allow(unused_imports)]
use log::{debug, error, info, warn};
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;

const TESTNET_WS_BASE_URL: &str = "wss://testnet.binance.vision/ws";
const MAINNET_WS_BASE_URL: &str = "wss://stream.binance.com:9443/ws";
const STATS_INTERVAL_SECS: u64 = 5;
const UNSOLICITED_PONG_INTERVAL_SECS: u64 = 180;

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

struct FixedConfig {
    use_testnet: bool,
    symbol: String,
}

fn parse_args() -> Result<FixedConfig, String> {
    let mut use_testnet = false;
    let mut symbol = "ethusdt".to_string();

    let args = env::args().collect::<Vec<_>>();
    let mut i = 1usize;

    while i < args.len() {
        match args[i].as_str() {
            "--testnet" => {
                use_testnet = true;
            }
            "--mainnet" => {
                use_testnet = false;
            }
            "--symbol" => {
                i += 1;
                let Some(value) = args.get(i) else {
                    return Err("Missing value for --symbol".to_string());
                };
                symbol = value.to_lowercase();
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                return Err(format!("Unknown option: {}", other));
            }
        }
        i += 1;
    }

    Ok(FixedConfig {
        use_testnet,
        symbol,
    })
}

fn print_help() {
    println!("Usage:");
    println!("  cargo run -p public_data_streaming --bin fixed_url_stream -- [options]");
    println!();
    println!("Options:");
    println!("  --symbol <symbol>   Stream symbol (default: ethusdt)");
    println!("  --testnet           Use spot testnet endpoint");
    println!("  --mainnet           Use spot mainnet endpoint (default)");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    settings::init_logging();

    let config = match parse_args() {
        Ok(cfg) => cfg,
        Err(err) => {
            error!("{}", err);
            print_help();
            return Ok(());
        }
    };

    let ws_base = if config.use_testnet {
        TESTNET_WS_BASE_URL
    } else {
        MAINNET_WS_BASE_URL
    };

    let stream = format!("{}@trade", config.symbol);
    let url = format!("{}/{}", ws_base, stream);
    let url = Url::parse(&url)?;

    info!("Starting fixed URL stream demo: {}", url);

    let (ws_stream, _) = tokio_tungstenite::connect_async(url).await?;
    info!("WebSocket handshake successful.");

    let (mut write, mut read) = ws_stream.split();
    let start_time = Instant::now();
    let mut message_count = 0usize;
    let mut last_message_time = Instant::now();
    let mut print_stats_interval = interval(Duration::from_secs(STATS_INTERVAL_SECS));
    let mut pong_interval = interval(Duration::from_secs(UNSOLICITED_PONG_INTERVAL_SECS));

    loop {
        tokio::select! {
            msg = read.next() => {
                if !handle_message(msg, &mut write, &mut message_count, &mut last_message_time).await? {
                    break;
                }
            }
            _ = print_stats_interval.tick() => {
                let elapsed = start_time.elapsed().as_secs_f64();
                info!(
                    "Messages received: {}, Frequency: {:.2} msg/s",
                    message_count,
                    message_count as f64 / elapsed
                );
            }
            _ = pong_interval.tick() => {
                debug!("Sending unsolicited pong heartbeat.");
                write.send(Message::Pong(vec![])).await?;
            }
        }
    }

    Ok(())
}

async fn handle_message(
    msg: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    write: &mut SplitSink<WsStream, Message>,
    message_count: &mut usize,
    last_message_time: &mut Instant,
) -> Result<bool, Box<dyn Error>> {
    match msg {
        Some(Ok(Message::Text(text))) => {
            handle_text_message(&text, message_count, last_message_time);
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

fn handle_text_message(message: &str, message_count: &mut usize, last_message_time: &mut Instant) {
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
            debug!("Subscription response: result={:?}, id={}", result, id);
        }
        Ok(models::BinanceMessage::Event(other)) => {
            debug!("Non-trade event: {:?}", other);
        }
        Ok(models::BinanceMessage::Other(other)) => {
            debug!("Other message: {:?}", other);
        }
        Err(e) => {
            warn!("Failed to deserialize message: {}, error: {}", message, e);
        }
    }
}
