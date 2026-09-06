// src/models.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct Channel {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Server {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub channels: Vec<Channel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageStatus { Sending, Delivered, Failed }

#[derive(Debug, Clone)]
pub struct DiscordMessage {
    pub nonce: String,
    pub author: String,
    pub content: String,
    pub timestamp: String,
    pub status: MessageStatus,
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    IncomingMessage(DiscordMessage),
    MessageSent { nonce: String, timestamp: String },
    MessageFailed { nonce: String },
    Terminal(crossterm::event::Event),
    GatewayClosed,
    SetSelfUsername(String),
    HttpTriggerTyping,
    HttpSendChat { nonce: String, text: String },
    FetchChannelHistory(String),
    LoadChannelHistory(Vec<DiscordMessage>),
    SwitchChannel(String),
    UpdateGatewayRtt { rtt_ms: u64, offset_ms: i64 },
    UpdateClockOffset(i64),
    ToggleTimestamp,
    ToggleLatency,
    ScrollChat(i32),
}

#[derive(Serialize)]
pub struct MessagePayload {
    pub content: String,
    pub nonce: String,
}

#[derive(Deserialize)]
pub struct GatewayPayload {
    pub op: u8,
    #[serde(default)]
    pub d: serde_json::Value,
    #[serde(default)]
    pub t: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscordApiMessage {
    pub id: String,
    #[serde(default)]
    pub nonce: Option<serde_json::Value>,
    #[serde(default)]
    pub content: String,
    pub author: DiscordApiAuthor,
    #[serde(default)]
    pub timestamp: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscordApiAuthor {
    pub username: String,
    #[serde(default)]
    pub global_name: Option<String>,
}
