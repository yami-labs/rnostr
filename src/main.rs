// rnostr/src/main.rs

use crate::{
    config::{ConfigManager, RelayConfig},
    state::{AppState, Setting},
    ws::create_router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::fmt;

pub mod state;
pub mod handler;
pub mod filter;
pub mod mesh;
pub mod message;
pub mod auth;
pub mod ws;
pub mod config;
pub mod error;
pub mod db;
pub mod upload;
pub mod storage;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    fmt::init();

    // 加载配置（支持热重载）
    let config_manager = ConfigManager::load("config.toml")?;
    let config = config_manager.get();

    // 监听配置变更（可选：动态调整某些运行时参数）
    let _reload_rx = config_manager.reload_tx.subscribe();
    // 注意：配置热重载功能暂时不启用，因为涉及复杂的生命周期管理

    // 将 RelayConfig 转换为 Setting
    let setting = Setting {
        listen_addr: config.listen_addr.clone(),
        siwe_domain: config.siwe.domain.clone(),
        siwe_uri: config.siwe.uri.clone(),
        siwe_chain_id: config.siwe.chain_id,
        max_subscriptions_per_conn: config.connection.max_subscriptions_per_conn,
        event_ttl_seconds: config.connection.event_ttl_seconds,
        blob_dir: config.blob_dir.clone(),
        // ... 其他配置项
    };

    let state = Arc::new(AppState::new(
        setting,
        Arc::new(mesh::DummyMesh) as Arc<dyn mesh::MeshProxy + Send + Sync>,
    ));

    let addr: SocketAddr = config.listen_addr.parse()?;

    let app = create_router(state);

    info!("Starting relay on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    
    info!("TLS disabled, listening on http://{}", addr);
    axum::serve(listener, app)
        .await?;

    Ok(())
}
