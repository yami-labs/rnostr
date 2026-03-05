// rnostr/src/ws.rs

use crate::{
    state::{AppState, AuthState, ConnectionState},
    auth::SiweAuthenticator,
    handler::handle_message,
};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

/// WebSocket 升级路由处理器
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// 处理单个 WebSocket 连接的全生命周期
async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let conn_id = Uuid::new_v4();
    info!("New WS connection: {}", conn_id);

    state.increment_connection();

    // 创建连接状态
    let conn_state = ConnectionState {
        id: conn_id,
        auth_state: AuthState::Unauthenticated,
        subscriptions: DashMap::new(),
    };
    state.connections.insert(conn_id, conn_state);

    // 分割 sender / receiver
    let (mut sender, mut receiver) = socket.split();

    // SIWE 认证器（从 config 读取）
    let setting = state.setting.read().clone();
    let authenticator = SiweAuthenticator::new(
        setting.siwe_domain.clone(),
        setting.siwe_uri.clone(),
        setting.siwe_chain_id,
    );

    // 发送 SIWE challenge（连接建立即要求认证）
    if let Ok(challenge_msg) = authenticator.create_challenge() {
        if sender.send(Message::Text(challenge_msg.into())).await.is_err() {
            error!("Failed to send SIWE challenge");
            let _ = sender.send(Message::Close(None)).await;
            return;
        }
    }

    // 主循环：处理客户端发来的消息
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Err(e) = handle_message(&text, &state, conn_id).await {
                    warn!("Message handling error: {}", e);
                }
            }
            Ok(Message::Binary(_)) => {
                warn!("Binary message received, ignoring");
            }
            Ok(Message::Close(_)) => {
                info!("Client closed connection: {}", conn_id);
                break;
            }
            Err(e) => {
                error!("WS error: {}", e);
                break;
            }
            _ => {}
        }
    }

    // 清理
    state.connections.remove(&conn_id);
    state.decrement_connection();
    info!("WS connection closed: {}", conn_id);
}

/// 创建 axum Router（供 main.rs 使用）
pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
        .route("/health", get(|| async { "OK" }))
        .with_state(state)
}
