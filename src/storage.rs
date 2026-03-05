// src/storage.rs

use crate::db::{Db, Tree, Scanner, TimeKey};
use crate::db::lmdb::Transaction;
use crate::filter::Filter;
use crate::error::Error;
use crate::db::MatchResult;
use serde_json::Value;
use std::sync::Arc;
use std::cmp::Ordering;
use std::ops::Bound;
use tracing::info;

/// 事件作为 TimeKey 的实现（用于 scanner 时间扫描）
#[derive(Clone, Debug)]
struct EventTimeKey {
    created_at: u64,
    id: String,  // 用于 cmp tie-breaker
}

impl TimeKey for EventTimeKey {
    fn time(&self) -> u64 {
        self.created_at
    }

    fn change_time(&self, _key: &[u8], time: u64) -> Vec<u8> {
        format!("{}:{}", time, self.id).into_bytes()
    }
}

impl PartialOrd for EventTimeKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(<Self as Ord>::cmp(self, other))
    }
}

impl Ord for EventTimeKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.time().cmp(&other.time()).then_with(|| self.id.cmp(&other.id))
    }
}

impl PartialEq for EventTimeKey {
    fn eq(&self, other: &Self) -> bool {
        self.time() == other.time() && self.id == other.id
    }
}

impl Eq for EventTimeKey {}

/// IM 专用存储封装
pub struct ImStorage {
    db: Arc<Db>,
    // 主事件树
    main_tree: Tree,
    // 房间索引树
    room_index_tree: Tree,
}

impl ImStorage {
    pub fn new(db_path: &str) -> Result<Self, Error> {
        let db = Db::open(db_path)?;

        // 打开主事件树（默认无名）
        let main_tree = db.open_tree(None, 0)?;

        // 打开房间索引树
        let room_index_tree = db.open_tree(Some("room_index"), 0)?;

        Ok(Self {
            db: Arc::new(db),
            main_tree,
            room_index_tree,
        })
    }

    /// 保存事件 + 更新房间索引
    pub async fn save_event(&self, event: &Value) -> Result<(), Error> {
        let id = event["id"].as_str().ok_or(Error::InvalidEvent("missing id".to_string()))?.to_string();
        let bytes = serde_json::to_vec(event)?;

        let mut writer = self.db.writer()?;

        // 存主事件
        writer.put(&self.main_tree, id.as_bytes(), &bytes)?;

        // 更新房间索引
        if let Some(room_id) = extract_room_id(event) {
            let created_at = event["created_at"].as_u64().ok_or(Error::InvalidEvent("missing created_at".to_string()))?;
            let index_key = format!("{}:{}", room_id, created_at);
            writer.put(&self.room_index_tree, index_key.as_bytes(), id.as_bytes())?;
        }

        writer.commit()?;

        info!("Stored IM event: {}", id);
        Ok(())
    }

    /// 根据 filter 查询事件
    pub async fn query_by_filter(
        &self,
        filter: &Filter,
    ) -> Result<Vec<Value>, Error> {
        let mut results = Vec::new();

        let reader = self.db.reader()?;

        // 优先房间索引扫描
        if let Some(room_values) = filter.tags.get("room") {
            if let Some(room_id) = room_values.first() {
                let prefix = format!("{}:", room_id);
                let mut iter = reader.iter_from(&self.room_index_tree, Bound::Included(prefix.as_bytes()), false);

                while let Some(item) = iter.next() {
                    let (key, value) = item?;
                    let event_id = String::from_utf8_lossy(value).to_string();

                    // 从主树取事件
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

        // 作者过滤或全扫描，使用 Scanner 优化时间范围
        let mut scanner: Scanner<EventTimeKey, _> = Scanner::new(
            reader.iter(&self.main_tree),
            vec![],  // key (dummy)
            vec![],  // prefix (full scan)
            false,   // reverse
            filter.since,
            filter.until,
            Box::new(|_scanner: &Scanner<EventTimeKey, Error>, (key, value): (&[u8], &[u8])| -> Result<MatchResult<EventTimeKey>, Error> {
                let event_id = String::from_utf8_lossy(key).to_string();
                // Extract created_at from the event value
                let created_at = if let Ok(event) = serde_json::from_slice::<Value>(value) {
                    event["created_at"].as_u64().unwrap_or(0)
                } else {
                    0
                };
                Ok(MatchResult::Found(EventTimeKey { created_at, id: event_id }))
            }),
        );

        while let Some(item) = scanner.next() {
            let key = item?;
            let event_id = key.id;
            if let Some(event_bytes) = reader.get(&self.main_tree, event_id.as_bytes())? {
                if let Ok(event) = serde_json::from_slice::<Value>(&event_bytes) {
                    if filter.matches(&event) {
                        results.push(event);
                    }
                }
            }
        }

        Ok(results)
    }

    /// GC 过期事件
    pub async fn gc_expired(&self, now: u64) -> Result<usize, Error> {
        let mut removed = 0;

        let reader = self.db.reader()?;

        let mut iter = reader.iter(&self.main_tree);

        while let Some(item) = iter.next() {
            let (key, value) = item?;
            let event_id = String::from_utf8_lossy(key).to_string();

            if let Ok(event) = serde_json::from_slice::<Value>(value) {
                let expire = event["tags"]
                    .as_array()
                    .and_then(|tags| tags.iter().find(|t| t[0].as_str() == Some("expire")))
                    .and_then(|t| t[1].as_u64());

                if let Some(exp) = expire {
                    if exp < now {
                        let mut writer = self.db.writer()?;
                        writer.del(&self.main_tree, event_id.as_bytes(), None)?;
                        writer.commit()?;
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