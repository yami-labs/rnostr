// src/storage.rs

use crate::db::{Db, LmdbResult, Reader, Tree, Writer};
use crate::filter::Filter;
use serde_json::Value;
use std::sync::Arc;
use tracing::{error, info, warn};

/// IM 专用存储封装（基于内置的 db::lmdb）
pub struct ImStorage {
    db: Arc<Db>,
    /// 默认事件树（主存储）
    main_tree: Tree,
    /// 房间索引树（可选加速）
    room_index_tree: Tree,
}

impl ImStorage {
    pub fn new(db_path: &str) -> LmdbResult<Self> {
        let db = Db::open(db_path)?;
        
        // 打开主事件树（无名称 = default）
        let main_tree = db.open_tree(None, 0)?;
        
        // 打开房间索引树（用固定名称）
        let room_index_tree = db.open_tree(Some("room_index"), 0)?;

        Ok(Self {
            db: Arc::new(db),
            main_tree,
            room_index_tree,
        })
    }

    /// 保存事件（存到主树 + 更新房间索引）
    pub async fn save_event(&self, event: &Value) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let id = event["id"].as_str().ok_or("missing id")?.to_string();
        let bytes = serde_json::to_vec(event)?;

        let mut writer = self.db.writer()?;
        
        // 存主事件（key = id, value = json bytes）
        writer.put(&self.main_tree, id.as_bytes(), &bytes)?;

        // 如果有 #room tag，更新房间索引
        if let Some(room_id) = extract_room_id(event) {
            let created_at = event["created_at"].as_u64().unwrap_or(0);
            let index_key = format!("{}:{}", created_at, id);
            writer.put(&self.room_index_tree, index_key.as_bytes(), &[])?; // value 可以为空，只用 key
        }

        writer.commit()?;
        info!("Stored IM event: {}", id);
        Ok(())
    }

    /// 根据 filter 查询事件（优先房间索引）
    pub async fn query_by_filter(
        &self,
        filter: &Filter,
    ) -> Result<Vec<Value>, Box<dyn std::error::Error + Send + Sync>> {
        let mut results = Vec::new();

        let reader = self.db.reader()?;

        // 优先走房间索引
        if let Some(room_values) = filter.tags.get("room") {
            if let Some(room_id) = room_values.first() {
                let prefix = format!("{}:", room_id); // 假设 key 是 "room_id:created_at:event_id"
                let mut iter = reader.iter_from(&self.room_index_tree, Bound::Included(prefix.as_bytes()), false);

                while let Some(item) = iter.next() {
                    let (key, _) = item?;
                    let key_str = String::from_utf8_lossy(key);
                    let parts: Vec<&str> = key_str.splitn(2, ':').collect();
                    if parts.len() != 2 { continue; }
                    let event_id = parts[1];

                    if let Some(event_bytes) = reader.get(&self.main_tree, event_id.as_bytes())? {
                        if let Ok(event) = serde_json::from_slice::<Value>(&event_bytes) {
                            if filter.matches(&event) {
                                results.push(event);
                            }
                        }
                    }
                }
                return Ok(results);
            }
        }

        // 作者过滤（次优先）
        if !filter.authors.is_empty() {
            // 假设主树有 pubkey 前缀索引（如果没有，需要在 save 时维护）
            // 这里简化：全扫描 + 过滤
            warn!("Author filter fallback to full scan");
            let mut iter = reader.iter(&self.main_tree);
            while let Some(item) = iter.next() {
                let (_, value) = item?;
                if let Ok(event) = serde_json::from_slice::<Value>(value) {
                    if filter.matches(&event) {
                        results.push(event);
                    }
                }
            }
            return Ok(results);
        }

        // 兜底全扫描
        warn!("Full scan query - performance warning");
        let mut iter = reader.iter(&self.main_tree);
        while let Some(item) = iter.next() {
            let (_, value) = item?;
            if let Ok(event) = serde_json::from_slice::<Value>(value) {
                if filter.matches(&event) {
                    results.push(event);
                }
            }
        }

        results.sort_by_key(|e| e["created_at"].as_u64().unwrap_or(0));
        Ok(results)
    }

    /// 删除单个事件（用于 GC）
    pub fn delete_event(&self, event_id: &str) -> LmdbResult<()> {
        let mut writer = self.db.writer()?;
        writer.del(&self.main_tree, event_id.as_bytes(), None)?;
        writer.commit()
    }

    /// GC 过期事件（从 gc.rs 调用）
    pub async fn gc_expired(&self, now: u64) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let mut removed = 0;
        let reader = self.db.reader()?;
        let mut iter = reader.iter(&self.main_tree);

        while let Some(item) = iter.next() {
            let (key_bytes, value_bytes) = item?;
            let event_id = String::from_utf8_lossy(key_bytes).to_string();

            if let Ok(event) = serde_json::from_slice::<Value>(value_bytes) {
                let expire = event["tags"]
                    .as_array()
                    .and_then(|tags| tags.iter().find(|t| t[0].as_str() == Some("expire")))
                    .and_then(|t| t[1].as_u64());

                if let Some(exp) = expire {
                    if exp < now {
                        self.delete_event(&event_id)?;
                        removed += 1;
                    }
                }
            }
        }

        if removed > 0 {
            info!("GC removed {} expired events", removed);
        }
        Ok(removed)
    }
}

// 辅助函数
fn extract_room_id(event: &Value) -> Option<String> {
    event["tags"]
        .as_array()
        .and_then(|tags| {
            tags.iter()
                .find(|t| t[0].as_str() == Some("room"))
                .and_then(|t| t[1].as_str().map(String::from))
        })
}