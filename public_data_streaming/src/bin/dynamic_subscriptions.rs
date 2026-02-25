use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use public_data_streaming::models;
use public_data_streaming::settings;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::io::{self, BufRead};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
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
const RECONNECT_DELAY_SECS: u64 = 3;

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Debug, Clone)]
enum WebSocketCommand {
    Subscribe(String),
    Unsubscribe(String),
    ListLocal,
    ListServer,
    Help,
    Quit,
}

#[derive(Debug)]
enum PendingRequest {
    Subscribe(Vec<String>),
    Unsubscribe(Vec<String>),
    ListServer,
}

struct DynamicWebSocket {
    ws_url: String,
    next_request_id: i64,
    desired_subscriptions: HashSet<String>,
    active_subscriptions: HashSet<String>,
    pending_requests: HashMap<i64, PendingRequest>,
    command_rx: mpsc::Receiver<WebSocketCommand>,
    shutdown_requested: bool,
}

impl DynamicWebSocket {
    fn new(
        use_testnet: bool,
        initial_subscriptions: Vec<String>,
        command_rx: mpsc::Receiver<WebSocketCommand>,
    ) -> Self {
        let ws_url = if use_testnet {
            TESTNET_WS_BASE_URL.to_string()
        } else {
            MAINNET_WS_BASE_URL.to_string()
        };

        let desired_subscriptions = initial_subscriptions
            .into_iter()
            .map(|topic| topic.to_lowercase())
            .collect::<HashSet<_>>();

        Self {
            ws_url,
            next_request_id: 1,
            desired_subscriptions,
            active_subscriptions: HashSet::new(),
            pending_requests: HashMap::new(),
            command_rx,
            shutdown_requested: false,
        }
    }

    async fn connect_and_listen(&mut self) -> Result<(), Box<dyn Error>> {
        while !self.shutdown_requested {
            let url = Url::parse(&self.ws_url)?;
            info!("Connecting to WebSocket endpoint: {}", self.ws_url);

            match tokio_tungstenite::connect_async(url).await {
                Ok((ws_stream, _)) => {
                    info!("WebSocket handshake successful.");
                    self.active_subscriptions.clear();
                    self.pending_requests.clear();

                    let (write, read) = ws_stream.split();
                    self.run_websocket_loop(write, read).await?;
                }
                Err(e) => {
                    error!("WebSocket connect error: {}", e);
                }
            }

            if !self.shutdown_requested {
                warn!("Disconnected; reconnecting in {}s...", RECONNECT_DELAY_SECS);
                tokio::time::sleep(Duration::from_secs(RECONNECT_DELAY_SECS)).await;
            }
        }

        Ok(())
    }

    async fn run_websocket_loop(
        &mut self,
        mut write: SplitSink<WsStream, Message>,
        mut read: SplitStream<WsStream>,
    ) -> Result<(), Box<dyn Error>> {
        let start_time = Instant::now();
        let mut message_count = 0usize;
        let mut last_message_time = Instant::now();
        let mut print_stats_interval = interval(Duration::from_secs(STATS_INTERVAL_SECS));
        let mut pong_interval = interval(Duration::from_secs(UNSOLICITED_PONG_INTERVAL_SECS));

        self.send_subscribe_request(
            &mut write,
            self.desired_subscriptions.iter().cloned().collect(),
        )
        .await?;

        loop {
            tokio::select! {
                cmd = self.command_rx.recv() => {
                    if !self.handle_command(cmd, &mut write).await? {
                        break;
                    }
                }
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

    async fn handle_command(
        &mut self,
        cmd: Option<WebSocketCommand>,
        write: &mut SplitSink<WsStream, Message>,
    ) -> Result<bool, Box<dyn Error>> {
        match cmd {
            Some(WebSocketCommand::Subscribe(stream)) => {
                let stream = normalize_stream(&stream);
                if !self.desired_subscriptions.insert(stream.clone()) {
                    info!("Already requested: {}", stream);
                    return Ok(true);
                }

                self.send_subscribe_request(write, vec![stream]).await?;
                Ok(true)
            }
            Some(WebSocketCommand::Unsubscribe(stream)) => {
                let stream = normalize_stream(&stream);
                if !self.desired_subscriptions.remove(&stream) {
                    warn!("Stream not in desired set: {}", stream);
                    return Ok(true);
                }

                self.send_unsubscribe_request(write, vec![stream]).await?;
                Ok(true)
            }
            Some(WebSocketCommand::ListLocal) => {
                self.list_local_subscriptions();
                Ok(true)
            }
            Some(WebSocketCommand::ListServer) => {
                self.send_list_server_request(write).await?;
                Ok(true)
            }
            Some(WebSocketCommand::Help) => {
                print_dynamic_help();
                Ok(true)
            }
            Some(WebSocketCommand::Quit) => {
                self.shutdown_requested = true;
                info!("Quit requested; closing websocket.");
                write.send(Message::Close(None)).await?;
                Ok(false)
            }
            None => {
                self.shutdown_requested = true;
                warn!("Command channel closed; shutting down.");
                Ok(false)
            }
        }
    }

    async fn handle_message(
        &mut self,
        msg: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
        write: &mut SplitSink<WsStream, Message>,
        message_count: &mut usize,
        last_message_time: &mut Instant,
    ) -> Result<bool, Box<dyn Error>> {
        match msg {
            Some(Ok(Message::Text(text))) => {
                self.handle_text_message(&text, message_count, last_message_time)
                    .await;
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

    async fn handle_text_message(
        &mut self,
        message: &str,
        message_count: &mut usize,
        last_message_time: &mut Instant,
    ) {
        let now = Instant::now();
        let time_since_last = now.duration_since(*last_message_time);
        *last_message_time = now;
        *message_count += 1;
        debug!("Time since last message: {:?}", time_since_last);

        let value: Value = match serde_json::from_str(message) {
            Ok(value) => value,
            Err(e) => {
                warn!("Failed to deserialize message: {}, error: {}", message, e);
                return;
            }
        };

        if value.get("id").is_some() {
            self.handle_api_response(value);
            return;
        }

        match serde_json::from_value::<models::BinanceMessage>(value) {
            Ok(models::BinanceMessage::Event(models::BinanceEvent::Trade(trade))) => {
                info!(
                    "Trade - Symbol: {}, Price: {}, Quantity: {}, Trade Time: {}",
                    trade.symbol, trade.price, trade.quantity, trade.trade_time
                );
            }
            Ok(models::BinanceMessage::Event(other)) => {
                debug!("Non-trade event: {:?}", other);
            }
            Ok(models::BinanceMessage::Other(other)) => {
                debug!("Other message: {:?}", other);
            }
            Ok(models::BinanceMessage::SubscriptionResponse { result, id }) => {
                debug!(
                    "Unmatched subscription response: id={}, result={:?}",
                    id, result
                );
            }
            Err(e) => {
                warn!("Failed to parse typed message, error: {}", e);
            }
        }
    }

    fn handle_api_response(&mut self, response: Value) {
        let Some(id) = response.get("id").and_then(Value::as_i64) else {
            warn!("Received response without numeric id: {:?}", response);
            return;
        };

        let Some(pending) = self.pending_requests.remove(&id) else {
            warn!(
                "Received response for unknown request id={}: {:?}",
                id, response
            );
            return;
        };

        if let Some(error_obj) = response.get("error") {
            match pending {
                PendingRequest::Subscribe(streams) => {
                    for stream in streams {
                        self.desired_subscriptions.remove(&stream);
                    }
                }
                PendingRequest::Unsubscribe(streams) => {
                    for stream in streams {
                        self.desired_subscriptions.insert(stream);
                    }
                }
                PendingRequest::ListServer => {}
            }

            error!("Request id={} failed: {:?}", id, error_obj);
            return;
        }

        match pending {
            PendingRequest::Subscribe(streams) => {
                for stream in streams {
                    self.active_subscriptions.insert(stream.clone());
                    info!("Subscription confirmed for {} (id={})", stream, id);
                }
            }
            PendingRequest::Unsubscribe(streams) => {
                for stream in streams {
                    self.active_subscriptions.remove(&stream);
                    info!("Unsubscription confirmed for {} (id={})", stream, id);
                }
            }
            PendingRequest::ListServer => {
                let server_list = response
                    .get("result")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                info!("Server subscriptions (id={}): {:?}", id, server_list);
            }
        }
    }

    async fn send_subscribe_request(
        &mut self,
        write: &mut SplitSink<WsStream, Message>,
        streams: Vec<String>,
    ) -> Result<(), Box<dyn Error>> {
        if streams.is_empty() {
            return Ok(());
        }

        let id = self.next_id();
        let msg = json!({
            "method": "SUBSCRIBE",
            "params": streams,
            "id": id
        });

        write.send(Message::Text(msg.to_string())).await?;

        let streams_for_state = msg
            .get("params")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        self.pending_requests
            .insert(id, PendingRequest::Subscribe(streams_for_state.clone()));
        info!("Sent SUBSCRIBE id={} streams={:?}", id, streams_for_state);

        Ok(())
    }

    async fn send_unsubscribe_request(
        &mut self,
        write: &mut SplitSink<WsStream, Message>,
        streams: Vec<String>,
    ) -> Result<(), Box<dyn Error>> {
        if streams.is_empty() {
            return Ok(());
        }

        let id = self.next_id();
        let msg = json!({
            "method": "UNSUBSCRIBE",
            "params": streams,
            "id": id
        });

        write.send(Message::Text(msg.to_string())).await?;

        let streams_for_state = msg
            .get("params")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        self.pending_requests
            .insert(id, PendingRequest::Unsubscribe(streams_for_state.clone()));
        info!("Sent UNSUBSCRIBE id={} streams={:?}", id, streams_for_state);

        Ok(())
    }

    async fn send_list_server_request(
        &mut self,
        write: &mut SplitSink<WsStream, Message>,
    ) -> Result<(), Box<dyn Error>> {
        let id = self.next_id();
        let msg = json!({
            "method": "LIST_SUBSCRIPTIONS",
            "id": id
        });

        write.send(Message::Text(msg.to_string())).await?;
        self.pending_requests.insert(id, PendingRequest::ListServer);
        info!("Sent LIST_SUBSCRIPTIONS id={}", id);

        Ok(())
    }

    fn list_local_subscriptions(&self) {
        let mut desired = self
            .desired_subscriptions
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        desired.sort();

        let mut active = self
            .active_subscriptions
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        active.sort();

        info!("Desired subscriptions: {:?}", desired);
        info!("Active subscriptions: {:?}", active);
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

    fn next_id(&mut self) -> i64 {
        let id = self.next_request_id;
        self.next_request_id += 1;
        id
    }
}

fn normalize_stream(stream: &str) -> String {
    stream.trim().to_lowercase()
}

fn print_dynamic_help() {
    info!("Dynamic mode commands:");
    info!("  addsub <stream>    - subscribe to a stream, e.g. btcusdt@trade");
    info!("  delsub <stream>    - unsubscribe from a stream");
    info!("  list               - show local desired/active subscriptions");
    info!("  listserver         - query server-side active subscriptions");
    info!("  help               - show command help");
    info!("  quit               - close websocket and exit");
}

fn spawn_stdin_command_reader(command_tx: mpsc::Sender<WebSocketCommand>) {
    std::thread::spawn(move || {
        let stdin = io::stdin();

        for line in stdin.lock().lines() {
            let Ok(input) = line else {
                continue;
            };

            let parts = input.split_whitespace().collect::<Vec<_>>();
            let cmd = match parts.as_slice() {
                ["addsub", stream] => Some(WebSocketCommand::Subscribe((*stream).to_string())),
                ["delsub", stream] => Some(WebSocketCommand::Unsubscribe((*stream).to_string())),
                ["list"] => Some(WebSocketCommand::ListLocal),
                ["listserver"] => Some(WebSocketCommand::ListServer),
                ["help"] => Some(WebSocketCommand::Help),
                ["quit"] => Some(WebSocketCommand::Quit),
                [] => None,
                _ => {
                    println!(
                        "Unknown command. Try: addsub <stream>, delsub <stream>, list, listserver, help, quit"
                    );
                    None
                }
            };

            if let Some(cmd) = cmd {
                let should_quit = matches!(cmd, WebSocketCommand::Quit);
                if command_tx.blocking_send(cmd).is_err() {
                    break;
                }

                if should_quit {
                    break;
                }
            }
        }
    });
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    settings::init_logging();

    info!("Starting Binance Public WebSocket Client (dynamic subscriptions)...");

    let use_testnet = false;
    let initial_streams = vec!["ethusdt@trade".to_string()];

    let (command_tx, command_rx) = mpsc::channel(100);
    spawn_stdin_command_reader(command_tx.clone());
    let _command_tx_guard = command_tx;
    print_dynamic_help();

    let mut ws_client = DynamicWebSocket::new(use_testnet, initial_streams, command_rx);
    ws_client.connect_and_listen().await
}
