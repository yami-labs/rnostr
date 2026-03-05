// relay-core/src/message.rs

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;

/// 客户端发给 relay 的消息（Incoming）
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "0", rename_all = "UPPERCASE")]
pub enum IncomingMessage {
    /// ["EVENT", <event>]
    Event(Value),

    /// ["REQ", <sub_id>, <filter1>, <filter2>, ...]
    Req(ReqPayload),

    /// ["CLOSE", <sub_id>]
    Close(String),

    /// ["AUTH", <event>]  ← 我们用这个承载 SIWE 签名事件
    Auth(Value),

    /// ["COUNT", <sub_id>, <filter>]
    Count(ReqPayload),

    /// 未知消息
    Unknown,
}

/// REQ 的 payload 部分（sub_id + 多个 filter）
#[derive(Debug, Clone, Deserialize)]
pub struct ReqPayload {
    pub sub_id: String,
    #[serde(flatten)]
    pub filters: Vec<crate::filter::Filter>,
}

/// relay 发给客户端的消息（Outgoing）
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum OutgoingMessage {
    /// ["EVENT", <event>]
    Event(Value),

    /// ["EOSE", <sub_id>]
    Eose(String),

    /// ["NOTICE", <message>]
    Notice(String),

    /// ["OK", <event_id>, <status>, <message>]
    Ok { id: String, status: bool, message: String },

    /// ["COUNT", <sub_id>, <count>]
    Count { sub_id: String, count: u64 },

    /// ["CLOSED", <sub_id>, <message>]
    Closed { sub_id: String, message: String },
}

impl OutgoingMessage {
    pub fn ok(id: &str, status: bool, message: &str) -> Self {
        OutgoingMessage::Ok {
            id: id.to_string(),
            status,
            message: message.to_string(),
        }
    }

    pub fn notice(message: &str) -> Self {
        OutgoingMessage::Notice(message.to_string())
    }

    pub fn eose(sub_id: &str) -> Self {
        OutgoingMessage::Eose(sub_id.to_string())
    }

    pub fn closed(sub_id: &str, message: &str) -> Self {
        OutgoingMessage::Closed {
            sub_id: sub_id.to_string(),
            message: message.to_string(),
        }
    }

    pub fn count(sub_id: &str, count: u64) -> Self {
        OutgoingMessage::Count {
            sub_id: sub_id.to_string(),
            count,
        }
    }

    /// 序列化为 WebSocket Text 消息
    pub fn to_text(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "[]".to_string())
    }
}

/// SIWE 专用消息辅助函数（可选，客户端可自定义格式）
pub fn siwe_challenge_message(domain: &str, uri: &str, nonce: &str, issued_at: &str) -> String {
    format!(
        r#"{} wants you to sign in with your Ethereum account:

URI: {}
Version: 1
Chain ID: 1
Nonce: {}
Issued At: {}"#,
        domain, uri, nonce, issued_at
    )
}

impl fmt::Display for IncomingMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IncomingMessage::Event(_) => write!(f, "EVENT"),
            IncomingMessage::Req(_) => write!(f, "REQ"),
            IncomingMessage::Close(_) => write!(f, "CLOSE"),
            IncomingMessage::Auth(_) => write!(f, "AUTH"),
            IncomingMessage::Count(_) => write!(f, "COUNT"),
            IncomingMessage::Unknown => write!(f, "UNKNOWN"),
        }
    }
}