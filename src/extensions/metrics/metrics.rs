// extensions/metrics/metrics.rs

use crate::state::AppState;
use metrics::{counter, describe_counter, gauge, histogram};
use std::sync::Arc;
use tokio::time::{interval, Duration};
use tracing::info;

/// Metrics 插件初始化（只在 feature = "metrics" 时编译）
#[cfg(feature = "metrics")]
pub fn init_metrics(state: &Arc<AppState>) {
    // 定义指标（describe 只调用一次）
    describe_counter!("relay_active_connections", "当前活跃 WebSocket 连接数");
    describe_counter!("relay_received_messages_total", "接收到的消息总数");
    describe_counter!("relay_im_events_processed", "处理的 IM 事件数");
    describe_counter!("relay_siwe_auth_success", "SIWE 认证成功次数");
    describe_counter!("relay_siwe_auth_failed", "SIWE 认证失败次数");
    describe_gauge!("relay_connection_count", "实时连接数");
    describe_histogram!("relay_message_latency_seconds", "消息处理延迟（秒）");

    info!("Metrics extension initialized (Prometheus counters enabled)");

    // 定时上报连接数（每 15 秒）
    let state_clone = state.clone();
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(15));
        loop {
            interval.tick().await;
            let count = state_clone.connection_count.load(std::sync::atomic::Ordering::Relaxed);
            gauge!("relay_connection_count").set(count as f64);
        }
    });
}

/// 在 handler 中记录事件处理成功（示例）
#[cfg(feature = "metrics")]
pub fn record_event_processed() {
    counter!("relay_im_events_processed").increment(1);
}

/// 记录 SIWE 认证结果
#[cfg(feature = "metrics")]
pub fn record_siwe_auth(success: bool) {
    if success {
        counter!("relay_siwe_auth_success").increment(1);
    } else {
        counter!("relay_siwe_auth_failed").increment(1);
    }
}

/// 记录消息处理延迟（示例）
#[cfg(feature = "metrics")]
pub fn observe_message_latency(duration_secs: f64) {
    histogram!("relay_message_latency_seconds").observe(duration_secs);
}

/// 无 metrics feature 时为空实现（零成本）
#[cfg(not(feature = "metrics"))]
pub fn init_metrics(_state: &Arc<AppState>) {}

#[cfg(not(feature = "metrics"))]
pub fn record_event_processed() {}

#[cfg(not(feature = "metrics"))]
pub fn record_siwe_auth(_success: bool) {}

#[cfg(not(feature = "metrics"))]
pub fn observe_message_latency(_duration_secs: f64) {}