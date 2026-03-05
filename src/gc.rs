use crate::storage::ImStorage;
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::time::{interval, Duration};
use tracing::{error, info, warn};

/// 垃圾回收任务配置
#[derive(Clone, Debug)]
pub struct GcConfig {
    /// 全局默认事件过期时间（秒）
    pub default_event_ttl: u64,
    /// 最小允许的事件 TTL（秒，防止用户设太短）
    pub min_event_ttl: u64,
    /// 文件存储目录
    pub blob_dir: PathBuf,
    /// GC 运行间隔（秒）
    pub interval_seconds: u64,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            default_event_ttl: 31_536_000,      // 1 年
            min_event_ttl: 15_552_000,          // 6 个月
            blob_dir: PathBuf::from("./blobs"),
            interval_seconds: 3600,             // 每小时
        }
    }
}

/// 启动垃圾回收定时任务
pub fn start_gc_task(storage: Arc<ImStorage>, config: GcConfig) {
    let storage_clone = storage.clone();

    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(config.interval_seconds));

        loop {
            interval.tick().await;

            let now = Utc::now().timestamp() as u64;

            info!("Starting GC cycle at {}", now);

            // 1. 清理过期事件
            match cleanup_expired_events(&storage_clone, now, &config).await {
                Ok(count) => info!("Removed {} expired events", count),
                Err(e) => error!("Event GC failed: {}", e),
            }

            // 2. 清理过期文件
            match cleanup_expired_blobs(&storage_clone, now, &config).await {
                Ok(count) => info!("Removed {} expired blobs", count),
                Err(e) => error!("Blob GC failed: {}", e),
            }
        }
    });
}

/// 清理过期事件
async fn cleanup_expired_events(
    storage: &ImStorage,
    now: u64,
    config: &GcConfig,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let mut removed = 0;

    // 获取所有事件（实际生产中应使用 cursor 批量扫描，避免内存爆炸）
    // 这里简化：假设 kv 提供了 scan_all（生产中应分批）
    let all_events = storage.inner.scan_all()?;

    for event_bytes in all_events {
        let event: Value = match serde_json::from_slice(&event_bytes) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let event_id = match event["id"].as_str() {
            Some(id) => id.to_string(),
            None => continue,
        };

        // 读取 expire tag（优先级最高）
        let expire_ts = event["tags"]
            .as_array()
            .and_then(|tags| {
                tags.iter()
                    .find(|t| t[0].as_str() == Some("expire"))
                    .and_then(|t| t[1].as_u64())
            })
            .or_else(|| {
                // 没有 tag，使用全局默认（但不能低于 min）
                let default = config.default_event_ttl;
                let min = config.min_event_ttl;
                Some(now.saturating_sub(default.max(min)))
            });

        if let Some(expire) = expire_ts {
            if expire < now {
                if let Err(e) = storage.inner.delete(&event_id) {
                    error!("Failed to delete event {}: {}", event_id, e);
                    continue;
                }
                removed += 1;
            }
        }
    }

    Ok(removed)
}

/// 清理过期文件（blob）
async fn cleanup_expired_blobs(
    storage: &ImStorage,
    now: u64,
    config: &GcConfig,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let mut removed = 0;

    // 遍历所有事件，检查是否有文件 + file_expire tag
    let all_events = storage.inner.scan_all()?;

    for event_bytes in all_events {
        let event: Value = match serde_json::from_slice(&event_bytes) {
            Ok(e) => e,
            Err(_) => continue,
        };

        // 假设文件 hash 或 id 在 tags 中，如 ["file", "hash_or_path"]
        let file_ident = event["tags"]
            .as_array()
            .and_then(|tags| {
                tags.iter()
                    .find(|t| t[0].as_str() == Some("file"))
                    .and_then(|t| t[1].as_str())
            });

        if let Some(file_name) = file_ident {
            // 检查文件专用过期时间
            let file_expire = event["tags"]
                .as_array()
                .and_then(|tags| {
                    tags.iter()
                        .find(|t| t[0].as_str() == Some("file_expire"))
                        .and_then(|t| t[1].as_u64())
                })
                .unwrap_or_else(|| {
                    // fallback 到事件 expire 或全局
                    event["tags"]
                        .as_array()
                        .and_then(|tags| {
                            tags.iter()
                                .find(|t| t[0].as_str() == Some("expire"))
                                .and_then(|t| t[1].as_u64())
                        })
                        .unwrap_or(now.saturating_sub(config.default_event_ttl))
                });

            if file_expire < now {
                let file_path = config.blob_dir.join(file_name);

                if file_path.exists() {
                    if let Err(e) = tokio::fs::remove_file(&file_path).await {
                        error!("Failed to delete blob {}: {}", file_path.display(), e);
                        continue;
                    }
                    removed += 1;
                    info!("Deleted expired blob: {}", file_path.display());
                }

                // 可选：删除事件中的 file tag（防止死链接）
                // 这里简化，不改事件本身
            }
        }
    }

    Ok(removed)
}