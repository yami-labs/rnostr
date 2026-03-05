// relay-core/src/filter.rs

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// IM 专用订阅过滤器（严格限制字段）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Filter {
    #[serde(skip)]
    pub id: Option<String>,

    // 只允许 IM 常用字段
    #[serde(default)]
    pub authors: Vec<String>,          // 消息作者

    #[serde(default)]
    pub kinds: Vec<u64>,               // 必须指定 1059, 14 等

    #[serde(default)]
    pub since: Option<u64>,

    #[serde(default)]
    pub until: Option<u64>,

    #[serde(default)]
    pub ids: Vec<String>,              // 精确事件 ID

    // 只支持 #e #p #room
    #[serde(default)]
    pub tags: std::collections::HashMap<String, Vec<String>>,
}

impl Filter {
    /// IM 兼容性检查（拒绝不支持的字段或过于宽泛的查询）
    pub fn is_im_compatible(&self) -> bool {
        // 必须有 kinds 限制（避免全量查询）
        if self.kinds.is_empty() {
            return false;
        }

        // 禁止其他标签
        for key in self.tags.keys() {
            if !matches!(key.as_str(), "e" | "p" | "room") {
                return false;
            }
        }

        // 至少要有 authors 或 #room 或 #p（避免无目标查询）
        if self.authors.is_empty() && !self.tags.contains_key("room") && !self.tags.contains_key("p") {
            return false;
        }

        true
    }

    /// 判断事件是否匹配此 filter（更严格的 IM 匹配）
    pub fn matches(&self, event: &Value) -> bool {
        // kind 必须匹配
        let kind = match event["kind"].as_u64() {
            Some(k) => k,
            None => return false,
        };
        if !self.kinds.is_empty() && !self.kinds.contains(&kind) {
            return false;
        }

        // authors
        if !self.authors.is_empty() {
            let pubkey = match event["pubkey"].as_str() {
                Some(p) => p,
                None => return false,
            };
            if !self.authors.iter().any(|a| a == pubkey) {
                return false;
            }
        }

        // 时间范围
        if let Some(ts) = event["created_at"].as_u64() {
            if let Some(since) = self.since {
                if ts < since { return false; }
            }
            if let Some(until) = self.until {
                if ts > until { return false; }
            }
        }

        // 精确 ID
        if !self.ids.is_empty() {
            let id = match event["id"].as_str() {
                Some(i) => i,
                None => return false,
            };
            if !self.ids.contains(&id.to_string()) {
                return false;
            }
        }

        // 标签匹配（#e #p #room）
        if let Some(tags) = event["tags"].as_array() {
            for (tag_key, values) in &self.tags {
                let tag_key_lower = tag_key.to_lowercase();
                let matched = tags.iter().any(|t| {
                    let tag_array = match t.as_array() {
                        Some(arr) => arr,
                        None => return false,
                    };
                    if tag_array.len() < 2 { return false; }
                    if tag_array[0].as_str() != Some(&tag_key_lower) { return false; }
                    if let Some(val) = tag_array[1].as_str() {
                        values.contains(&val.to_string())
                    } else {
                        false
                    }
                });
                if !matched {
                    return false;
                }
            }
        }

        true
    }

    pub fn from_req_array(arr: &[Value]) -> Option<Self> {
        if arr.len() < 2 { return None; }
        let id = arr[1].as_str().map(String::from);
        let filter_val = arr.get(2)?;
        let mut filter: Filter = serde_json::from_value(filter_val.clone()).ok()?;
        filter.id = id;
        Some(filter)
    }
}