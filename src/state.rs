use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::sync::Mutex;
use uuid::Uuid;
use std::sync::Arc;
use std::path::PathBuf;

// ИСПРАВЛЕНИЕ: Добавлены необходимые директивы.
#[derive(Serialize, Deserialize, Clone, Debug, Copy, PartialEq)]
pub enum ObfuscationPattern {
    Sunshine,
    Starfall,
}

#[derive(Serialize, Deserialize, Clone, Debug, Copy, PartialEq)]
pub enum NoiseLevel {
    Off,
    Slow,
    Medium,
    Fast,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FileContent {
    pub filename: String,
    #[serde(with = "base64_serde", default, skip_serializing_if = "Vec::is_empty")]
    pub data: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Uuid>,
}

// ИСПРАВЛЕНИЕ: Добавлены необходимые директивы.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", content = "payload")]
pub enum MessageContent {
    Text(String),
    File(FileContent),
}

// --- Структуры для API-запросов (перенесены из web.rs) ---

#[derive(Deserialize)]
pub struct AddKeyPayload { pub key: String }

#[derive(Deserialize)]
pub struct SendMessagePayload {
    pub target_addr: String,
    pub key: String,
    pub pattern: ObfuscationPattern,
    pub content: MessageContent,
}

#[derive(Deserialize)]
pub struct SetNoisePayload {
    pub level: NoiseLevel,
}


// --- Остальной код файла без изменений ---

mod base64_serde {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use base64::{Engine as _, engine::general_purpose};
    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> where S: Serializer {
        serializer.serialize_str(&general_purpose::STANDARD.encode(bytes))
    }
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error> where D: Deserializer<'de> {
        let s = String::deserialize(deserializer)?;
        general_purpose::STANDARD.decode(s.as_bytes()).map_err(serde::de::Error::custom)
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct DecryptedMessage {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub sender: SocketAddr,
    pub content: MessageContent,
    pub decrypted_with_key: String,
    pub decrypted_with_pattern: ObfuscationPattern,
}

#[derive(Debug)]
pub enum TransmitCommand {
    SendMessage {
        target_addr: SocketAddr,
        key: String,
        pattern: ObfuscationPattern,
        content: MessageContent,
    },
    SetNoiseLevel(NoiseLevel),
}

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "event", content = "data")]
pub enum WsNotification {
    FullState {
        keys: Vec<String>,
        messages: Vec<DecryptedMessage>,
        stats: AppStats,
    },
    NewMessage(DecryptedMessage),
    NoisePacket {
        sender: SocketAddr,
        size: usize,
    },
    KeyUpdate(Vec<String>),
    StatsUpdate(AppStats),
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, Copy)]
pub struct AppStats {
    pub packets_sent: u64,
    pub packets_received: u64,
    pub noise_packets_sent: u64,
    pub messages_decrypted: u64,
}

pub struct AppState {
    pub keys: Vec<String>,
    pub messages: Vec<DecryptedMessage>,
    pub received_files: HashMap<Uuid, (String, Vec<u8>)>,
    pub reassembly_buffer: HashMap<(SocketAddr, u32), HashMap<u32, Vec<u8>>>,
    pub downloads_path: PathBuf,
    pub stats: AppStats,
}

impl AppState {
    pub fn new(downloads_path: PathBuf) -> Self {
        Self {
            keys: Vec::new(),
            messages: Vec::new(),
            received_files: HashMap::new(),
            reassembly_buffer: HashMap::new(),
            downloads_path,
            stats: AppStats::default(),
        }
    }
}

pub type SharedState = Arc<Mutex<AppState>>;