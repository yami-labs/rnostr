// rnostr/src/handler.rs

use crate::state::{AppState, AuthState, BroadcastMessage};
use crate::message::{IncomingMessage, OutgoingMessage};
use crate::filter::Filter;
use serde_json::Value;
use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

/// 处理一条 incoming WebSocket 文本消息
pub async fn handle_message(
    text: &str,
    state: &Arc<AppState>,
    conn_id: Uuid,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let msg: IncomingMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            warn!("Invalid JSON: {}", e);
            return Ok(());
        }
    };

    match msg {
        IncomingMessage::Event(event) => {
            handle_event(event, state, conn_id).await?;
        }
        IncomingMessage::Req(sub) => {
            handle_req(sub, state, conn_id).await?;
        }
        IncomingMessage::Close(sub_id) => {
            handle_close(sub_id, state, conn_id).await?;
        }
        IncomingMessage::Auth(auth_event) => {
            handle_auth(auth_event, state, conn_id).await?;
        }
        IncomingMessage::Count(sub) => {
            handle_count(sub, state, conn_id).await?;
        }
        IncomingMessage::Unknown => {
            warn!("Unsupported message type");
        }
    }

    Ok(())
}

/// 处理 EVENT 消息（只允许 DM/房间相关）
async fn handle_event(
    mut event: Value,
    state: &Arc<AppState>,
    conn_id: Uuid,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let conn_opt = state.connections.get(&conn_id);
    let conn = conn_opt.ok_or("connection gone")?;

    // 1. 必须已认证
    if !conn.auth_state.is_authenticated() {
        return Err("unauthenticated".into());
    }

    // 2. 提取 kind
    let kind = event["kind"].as_u64().ok_or("missing kind")?;
    
    // 只允许 IM 相关 kind（可扩展）
    if kind != 1059 && kind != 14 && !is_room_control_kind(kind) {
        return Err("non-im event rejected".into());
    }

    // 3. 验证签名、过期 tag 等（这里简化）

    // 4. 如果是房间消息，更新 room_index
    if let Some(room_id) = extract_room_id(&event) {
        if let Some(mut index) = state.room_index.get_mut(&room_id) {
            let created_at = event["created_at"].as_u64().unwrap_or(0);
            let event_id = event["id"].as_str().unwrap_or("").to_string();
            index.push((created_at, event_id));
            // 按时间排序（可选优化）
            index.sort_by_key(|&(ts, _)| ts);
        }
    }

    // 5. 本地广播
    state.broadcast.send(BroadcastMessage::Event(event.clone()))?;

    // 6. 如果本地可能无订阅者，且是房间消息，尝试 P2P 转发
    if is_room_message(&event) && !has_local_subscribers_for_room(&event, state).await? {
        info!("No local subscribers for room, forwarding via mesh");
        if let Err(e) = state.mesh_proxy.forward_event(event).await {
            error!("Mesh forward failed: {}", e);
        }
    }

    Ok(())
}

/// 处理 REQ 订阅请求（限制 filter 字段）
async fn handle_req(
    sub: crate::message::ReqPayload,
    state: &Arc<AppState>,
    conn_id: Uuid,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut conn = state.connections.get_mut(&conn_id).ok_or("conn gone")?;

    // 限制 filter 只支持 IM 常用字段
    for filter in &sub.filters {
        if !filter.is_im_compatible() {
            return Err("unsupported filter fields for IM relay".into());
        }
    }

    // 订阅上限检查
    let setting = state.setting.read();
    if conn.subscriptions.len() >= setting.max_subscriptions_per_conn {
        return Err("too many subscriptions".into());
    }

    for filter in &sub.filters {
        conn.subscriptions.insert(sub.sub_id.clone(), filter.clone());
    }

    Ok(())
}

/// 处理 CLOSE
async fn handle_close(
    sub_id: String,
    state: &Arc<AppState>,
    conn_id: Uuid,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(mut conn) = state.connections.get_mut(&conn_id) {
        conn.subscriptions.remove(&sub_id);
    }
    Ok(())
}

/// 处理 AUTH（SIWE 占位）
async fn handle_auth(
    auth_event: Value,
    state: &Arc<AppState>,
    conn_id: Uuid,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("SIWE auth placeholder: event received");

    if let Some(mut conn) = state.connections.get_mut(&conn_id) {
        conn.auth_state = AuthState::SiweAddress("0xplaceholder".to_string()); // 临时
    }

    Ok(())
}

/// 处理 COUNT（简化版）
async fn handle_count(
    sub: crate::message::ReqPayload,
    state: &Arc<AppState>,
    conn_id: Uuid,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    Ok(())
}

// 辅助函数（示例）
fn is_room_message(event: &Value) -> bool {
    event["tags"]
        .as_array()
        .and_then(|tags| tags.iter().find(|t| t[0].as_str() == Some("room")))
        .is_some()
}

fn extract_room_id(event: &Value) -> Option<String> {
    event["tags"]
        .as_array()
        .and_then(|tags| {
            tags.iter()
                .find(|t| t[0].as_str() == Some("room"))
                .and_then(|t| t[1].as_str().map(String::from))
        })
}

async fn has_local_subscribers_for_room(
    event: &Value,
    state: &Arc<AppState>,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    Ok(false)
}

fn is_room_control_kind(kind: u64) -> bool {
    // NIP-29 房间管理事件 kind 范围（示例）
    (39000..=39999).contains(&kind)
}
