use futures_util::{SinkExt, StreamExt};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::time::{interval, Duration};
use tokio_tungstenite::{accept_async, connect_async};
use tokio_tungstenite::tungstenite::protocol::Message;

#[derive(Deserialize, Serialize)]
#[serde(tag = "type")]
enum ProxyAction {
    SendMessage { channel_id: String, content: String, nonce: String },
    SendTyping { channel_id: String },
    FetchHistory { channel_id: String, limit: u32 },
    SubscribeChannel { channel_id: String },
    Ping,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
enum ProxyResponse {
    Ack { nonce: String },
    MessageResult { nonce: String, success: bool, error: Option<String> },
    ChannelHistory { channel_id: String, messages: serde_json::Value },
    GatewayEvent { event_type: String, data: serde_json::Value },
    Pong,
}

#[derive(Deserialize)]
struct GatewayPayload {
    op: u8,
    #[serde(default)]
    d: serde_json::Value,
    #[serde(default)]
    t: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let discord_token = env::var("DISCORD_TOKEN")
        .expect("DISCORD_TOKEN environment variable must be set on the proxy server");

    let port = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("0.0.0.0:{}", port);

    let listener = TcpListener::bind(&addr).await?;
    println!("Remote proxy server listening on ws://{}", addr);

    let (gw_tx, _) = broadcast::channel::<ProxyResponse>(256);

    // Spawn Discord Gateway loop on the proxy server
    let gw_broadcast_tx = gw_tx.clone();
    let token_clone = discord_token.clone();
    tokio::spawn(async move {
        let gw_url = "wss://gateway.discord.gg/?v=10&encoding=json";
        loop {
            if let Ok((ws, _)) = connect_async(gw_url).await {
                let (mut write, mut read) = ws.split();

                if let Some(Ok(Message::Text(t))) = read.next().await {
                    if let Ok(p) = serde_json::from_str::<GatewayPayload>(&t) {
                        if p.op == 10 {
                            let heartbeat_interval = p.d["heartbeat_interval"].as_u64().unwrap_or(41250);
                            let identify = serde_json::json!({
                                "op": 2,
                                "d": {
                                    "token": token_clone,
                                    "properties": {
                                        "$os": "linux",
                                        "$browser": "chrome",
                                        "$device": "pc"
                                    }
                                }
                            });
                            let _ = write.send(Message::Text(identify.to_string())).await;

                            let shared_w = Arc::new(tokio::sync::Mutex::new(write));
                            let hb_w = Arc::clone(&shared_w);

                            // Heartbeat loop
                            tokio::spawn(async move {
                                let mut hb_timer = interval(Duration::from_millis(heartbeat_interval));
                                loop {
                                    hb_timer.tick().await;
                                    let hb_payload = serde_json::json!({ "op": 1, "d": null }).to_string();
                                    let mut w = hb_w.lock().await;
                                    if w.send(Message::Text(hb_payload)).await.is_err() {
                                        break;
                                    }
                                }
                            });

                            while let Some(Ok(Message::Text(msg_text))) = read.next().await {
                                if let Ok(pay) = serde_json::from_str::<GatewayPayload>(&msg_text) {
                                    if pay.op == 0 {
                                        let event_type = pay.t.unwrap_or_default();
                                        let response = ProxyResponse::GatewayEvent {
                                            event_type,
                                            data: pay.d,
                                        };
                                        let _ = gw_broadcast_tx.send(response);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });

    let mut default_headers = HeaderMap::new();
    default_headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    default_headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
    default_headers.insert(USER_AGENT, HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36"));

    let http_client = Arc::new(
        reqwest::Client::builder()
            .tcp_nodelay(true)
            .tcp_keepalive(Duration::from_secs(15))
            .default_headers(default_headers)
            .build()?
    );

    while let Ok((stream, _)) = listener.accept().await {
        let _ = stream.set_nodelay(true);
        let token = discord_token.clone();
        let client = Arc::clone(&http_client);
        let mut client_gw_rx = gw_tx.subscribe();

        tokio::spawn(async move {
            if let Ok(ws_stream) = accept_async(stream).await {
                let (mut write, mut read) = ws_stream.split();
                let write_arc = Arc::new(tokio::sync::Mutex::new(write));

                let subscribed_cid = Arc::new(tokio::sync::RwLock::new(String::new()));
                let client_cid_ref = Arc::clone(&subscribed_cid);

                // Forward Gateway events from Discord to client with channel filtering
                let gw_writer = Arc::clone(&write_arc);
                tokio::spawn(async move {
                    while let Ok(event) = client_gw_rx.recv().await {
                        let should_send = match &event {
                            ProxyResponse::GatewayEvent { event_type, data } => {
                                if event_type == "READY" {
                                    true
                                } else if event_type == "MESSAGE_CREATE" {
                                    let active_cid = client_cid_ref.read().await;
                                    data["channel_id"].as_str() == Some(active_cid.as_str())
                                } else {
                                    false
                                }
                            }
                            _ => true,
                        };

                        if should_send {
                            if let Ok(json_str) = serde_json::to_string(&event) {
                                let mut w = gw_writer.lock().await;
                                if w.send(Message::Text(json_str)).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });

                // Periodic ping keepalive
                let ping_writer = Arc::clone(&write_arc);
                let mut ping_interval = interval(Duration::from_secs(15));
                tokio::spawn(async move {
                    loop {
                        ping_interval.tick().await;
                        let mut w = ping_writer.lock().await;
                        if w.send(Message::Ping(vec![])).await.is_err() {
                            break;
                        }
                    }
                });

                while let Some(Ok(msg)) = read.next().await {
                    match msg {
                        Message::Ping(data) => {
                            let mut w = write_arc.lock().await;
                            let _ = w.send(Message::Pong(data)).await;
                        }
                        Message::Text(text) => {
                            if let Ok(action) = serde_json::from_str::<ProxyAction>(&text) {
                                match action {
                                    ProxyAction::SubscribeChannel { channel_id } => {
                                        *subscribed_cid.write().await = channel_id;
                                    }
                                    ProxyAction::Ping => {
                                        let resp = serde_json::to_string(&ProxyResponse::Pong).unwrap();
                                        let mut w = write_arc.lock().await;
                                        let _ = w.send(Message::Text(resp)).await;
                                    }
                                    ProxyAction::SendMessage { channel_id, content, nonce } => {
                                        let ack_json = serde_json::to_string(&ProxyResponse::Ack { nonce: nonce.clone() }).unwrap();
                                        let w_arc_ack = Arc::clone(&write_arc);
                                        tokio::spawn(async move {
                                            let mut w = w_arc_ack.lock().await;
                                            let _ = w.send(Message::Text(ack_json)).await;
                                        });

                                        let url = format!("https://discord.com/api/v10/channels/{}/messages", channel_id);
                                        let payload = serde_json::json!({ "content": content, "nonce": nonce });
                                        let client_ref = Arc::clone(&client);
                                        let token_ref = token.clone();
                                        let w_arc_res = Arc::clone(&write_arc);

                                        tokio::spawn(async move {
                                            let res = client_ref.post(&url)
                                                .header("Authorization", &token_ref)
                                                .header("Content-Type", "application/json")
                                                .json(&payload)
                                                .send()
                                                .await;

                                            let response = match res {
                                                Ok(resp) if resp.status().is_success() => ProxyResponse::MessageResult {
                                                    nonce,
                                                    success: true,
                                                    error: None,
                                                },
                                                Ok(resp) => {
                                                    let err = resp.text().await.unwrap_or_else(|_| "Rejected".to_string());
                                                    ProxyResponse::MessageResult {
                                                        nonce,
                                                        success: false,
                                                        error: Some(err),
                                                    }
                                                }
                                                Err(e) => ProxyResponse::MessageResult {
                                                    nonce,
                                                    success: false,
                                                    error: Some(e.to_string()),
                                                },
                                            };

                                            if let Ok(json_resp) = serde_json::to_string(&response) {
                                                let mut w = w_arc_res.lock().await;
                                                let _ = w.send(Message::Text(json_resp)).await;
                                            }
                                        });
                                    }
                                    ProxyAction::SendTyping { channel_id } => {
                                        let url = format!("https://discord.com/api/v10/channels/{}/typing", channel_id);
                                        let client_ref = Arc::clone(&client);
                                        let token_ref = token.clone();
                                        tokio::spawn(async move {
                                            let _ = client_ref.post(&url)
                                                .header("Authorization", &token_ref)
                                                .header("Content-Length", "0")
                                                .send()
                                                .await;
                                        });
                                    }
                                    ProxyAction::FetchHistory { channel_id, limit } => {
                                        let url = format!("https://discord.com/api/v10/channels/{}/messages?limit={}", channel_id, limit);
                                        let client_ref = Arc::clone(&client);
                                        let token_ref = token.clone();
                                        let w_arc_hist = Arc::clone(&write_arc);

                                        tokio::spawn(async move {
                                            let res = client_ref.get(&url)
                                                .header("Authorization", &token_ref)
                                                .send()
                                                .await;

                                            if let Ok(resp) = res {
                                                if let Ok(json_data) = resp.json::<serde_json::Value>().await {
                                                    let resp_struct = ProxyResponse::ChannelHistory {
                                                        channel_id,
                                                        messages: json_data,
                                                    };
                                                    if let Ok(json_resp) = serde_json::to_string(&resp_struct) {
                                                        let mut w = w_arc_hist.lock().await;
                                                        let _ = w.send(Message::Text(json_resp)).await;
                                                    }
                                                }
                                            }
                                        });
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        });
    }

    Ok(())
}
