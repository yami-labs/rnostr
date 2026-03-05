// src/error.rs

use thiserror::Error;

/// 项目统一错误类型
#[derive(Error, Debug)]
pub enum AppError {
    // ==================== 通用错误 ====================
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization/deserialization failed: {0}")]
    Json(#[from] serde_json::Error),

    #[error("UUID error: {0}")]
    Uuid(#[from] uuid::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Internal server error: {0}")]
    Internal(String),

    // ==================== WebSocket / Axum 相关 ====================
    #[error("WebSocket error: {0}")]
    Ws(#[from] axum::Error),

    #[error("Multipart error: {0}")]
    Multipart(String),

    #[error("Invalid WebSocket message format")]
    InvalidWsMessage,

    #[error("Connection closed unexpectedly")]
    ConnectionClosed,

    // ==================== 认证相关 ====================
    #[error("Authentication required")]
    AuthRequired,

    #[error("SIWE verification failed: {0}")]
    SiweVerification(String),

    #[error("Invalid SIWE message format")]
    InvalidSiweMessage,

    #[error("SIWE domain or URI mismatch")]
    SiweMismatch,

    #[error("SIWE signature invalid")]
    InvalidSignature,

    // ==================== 协议 / 事件相关 ====================
    #[error("Invalid Nostr event: {0}")]
    InvalidEvent(String),

    #[error("Event kind not allowed in IM relay (only DM/room supported)")]
    InvalidEventKind,

    #[error("Missing required tag: {0}")]
    MissingTag(String),

    #[error("Unsupported filter fields for IM relay")]
    UnsupportedFilter,

    #[error("Too many subscriptions (max: {0})")]
    TooManySubscriptions(usize),

    #[error("Event expired or invalid timestamp")]
    EventExpired,

    // ==================== 存储 / LMDB 相关 ====================
    #[error("LMDB error: {0}")]
    Lmdb(String),

    #[error("Nul error: {0}")]
    NulError(#[from] std::ffi::NulError),

    #[error("Storage operation failed: {0}")]
    Storage(String),

    #[error("Event not found")]
    EventNotFound,

    // ==================== 文件上传相关 ====================
    #[error("File too large (max 50MB)")]
    FileTooLarge,

    #[error("Unsupported file type")]
    UnsupportedFileType,

    #[error("Executable files are not allowed")]
    ExecutableNotAllowed,

    #[error("MIME type mismatch")]
    MimeMismatch,

    // ==================== P2P mesh 相关 ====================
    #[error("Mesh initialization failed: {0}")]
    MeshInit(String),

    #[error("Mesh forwarding failed: {0}")]
    MeshForward(String),

    #[error("Mesh query failed")]
    MeshQueryFailed,

    #[error("Gossipsub error: {0}")]
    Gossipsub(String),

    #[error("Kademlia error: {0}")]
    Kademlia(String),

    #[error("mDNS error: {0}")]
    Mdns(String),

    #[error("P2P network error: {0}")]
    Network(String),

    #[error("Peer not found: {0}")]
    PeerNotFound(String),

    #[error("Not enough peers for backup operation")]
    InsufficientPeers,

    // ==================== GC / 定时任务 ====================
    #[error("GC operation failed: {0}")]
    GcFailed(String),
}

// 统一错误类型别名
pub type Error = AppError;

// 如果未来有其他错误来源，也可以继续加 From 实现