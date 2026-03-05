// rnostr/src/state.rs

use crate::filter::Filter;           // 后面会定义
use crate::mesh::MeshProxy;          // P2P 转发接口
use crate::message::OutgoingMessage;
use dashmap::DashMap;
use parking_lot::RwLock;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::broadcast;
use uuid::Uuid;

/// 全局应用状态（所有连接共享）
#[derive(Clone)]
pub struct AppState {
    /// 所有活跃 WebSocket 连接的状态
    pub connections: Arc<DashMap<Uuid, ConnectionState>>,
    
    /// 广播通道，用于本地推送匹配的事件
    pub broadcast: broadcast::Sender<BroadcastMessage>,
    
    /// 配置（热重载支持）
    pub setting: Arc<RwLock<Setting>>,
    
    /// 当前连接数（用于监控）
    pub connection_count: Arc<AtomicU64>,
    
    /// P2P mesh 转发代理（ libp2p 或 dummy 实现）
    pub mesh_proxy: Arc<dyn MeshProxy + Send + Sync>,
    
    /// 房间事件快速索引（加速房间历史查询）
    /// key: room_id, value: Vec<(created_at, event_id)>
    pub room_index: Arc<DashMap<String, Vec<(u64, String)>>>,
}

/// 每个 WebSocket 连接的状态
#[derive(Clone)]
pub struct ConnectionState {
    pub id: Uuid,
    pub auth_state: AuthState,
    /// 当前连接订阅的所有订阅（sub_id → Filter）
    pub subscriptions: DashMap<String, Filter>,
}

#[derive(Clone, Debug)]
pub enum AuthState {
    Unauthenticated,
    SiweAddress(String),   // Ethereum 地址 (checksummed hex)
    Pubkey(String),        // nostr pubkey hex（兼容模式，可选）
}

impl AuthState {
    pub fn is_authenticated(&self) -> bool {
        !matches!(self, AuthState::Unauthenticated)
    }

    pub fn address_or_pubkey(&self) -> Option<String> {
        match self {
            AuthState::SiweAddress(addr) => Some(addr.clone()),
            AuthState::Pubkey(pk) => Some(pk.clone()),
            _ => None,
        }
    }
}

/// 广播的消息类型
#[derive(Clone, Debug)]
pub enum BroadcastMessage {
    /// 新事件需要推送给匹配的订阅者
    Event(Value),
    /// 订阅结果（EOSE、COUNT 等）
    SubscriptionResult(String, OutgoingMessage),
}

/// 配置结构体（从 config.toml 加载）
#[derive(Clone, Debug, Default)]
pub struct Setting {
    pub listen_addr: String,
    pub siwe_domain: String,
    pub siwe_uri: String,
    pub siwe_chain_id: u64,
    pub max_subscriptions_per_conn: usize,
    pub event_ttl_seconds: u64, // 过期时间（默认 1 年 = 31536000）
    // ... 其他配置
}

impl AppState {
    pub fn new(setting: Setting, mesh_proxy: Arc<dyn MeshProxy + Send + Sync>) -> Self {
        let (tx, _) = broadcast::channel(16384); // 缓冲大小可调
        Self {
            connections: Arc::new(DashMap::new()),
            broadcast: tx,
            setting: Arc::new(RwLock::new(setting)),
            connection_count: Arc::new(AtomicU64::new(0)),
            mesh_proxy,
            room_index: Arc::new(DashMap::new()),
        }
    }

    pub fn increment_connection(&self) {
        self.connection_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn decrement_connection(&self) {
        self.connection_count.fetch_sub(1, Ordering::Relaxed);
    }
}