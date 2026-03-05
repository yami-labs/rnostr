// relay-core/src/upload.rs

use axum::{
    extract::State,
    response::Json,
};
use axum_extra::extract::Multipart;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;
use tracing::info;
use mime_guess::from_path;
use std::path::PathBuf;
use serde_json::{json, Value};
use crate::{
    error::AppError,
    state::AppState
};


/// 支持的 MIME 类型白名单（从工具结果提取 + 常见类型）
const ALLOWED_MIME_TYPES: &[&str] = &[
    // 图片
    "image/jpeg", "image/png", "image/gif", "image/svg+xml", "image/webp", "image/bmp", "image/tiff", "image/x-icon",
    // 视频
    "video/mp4", "video/webm", "video/quicktime", "video/mpeg", "video/x-msvideo", "video/x-flv",
    // 压缩文件
    "application/zip", "application/gzip", "application/x-rar-compressed", "application/x-tar", "application/x-7z-compressed", "application/x-bzip2",
    // 文档
    "application/pdf", "application/msword", "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.ms-excel", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "text/plain", "text/csv", "application/rtf", "application/vnd.oasis.opendocument.text", "application/vnd.oasis.opendocument.spreadsheet",
];

/// 黑名单扩展名（可执行等）
const BLOCKED_EXTENSIONS: &[&str] = &["exe", "bat", "dll", "msi", "com", "cmd", "sh", "bin", "app", "dmg", "jar"];

/// 文件上传处理器
pub async fn upload_handler(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<Value>, AppError> {
    while let Some(field) = multipart.next_field().await.map_err(|e| AppError::Multipart(e.to_string()))? {
        let file_name = field.file_name().map(|n| n.to_string()).ok_or(AppError::Internal("missing file name".to_string()))?;
        let content_type = field.content_type().map(|t| t.to_string()).unwrap_or_else(|| "application/octet-stream".to_string());

        // 1. MIME 类型白名单检查
        if !ALLOWED_MIME_TYPES.contains(&content_type.as_str()) {
            return Err(AppError::UnsupportedFileType);
        }

        // 2. 扩展名黑名单检查
        let ext = PathBuf::from(&file_name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if BLOCKED_EXTENSIONS.contains(&ext.as_str()) {
            return Err(AppError::ExecutableNotAllowed);
        }

        // 3. 额外 MIME 猜检查（防伪造）
        if let Some(guessed_mime) = from_path(&file_name).first() {
            if guessed_mime.as_ref() != content_type {
                return Err(AppError::MimeMismatch);
            }
        }

        // 4. 读取数据流 + 大小验证
        let mut size: usize = 0;
        let mut hasher = Sha256::new();
        let uuid = Uuid::new_v4().to_string();
        let file_ext = ext.clone();
        let file_path = state.setting.read().blob_dir.parse::<PathBuf>().map_err(|e| AppError::Internal(e.to_string()))?.join(format!("{}.{}", uuid, file_ext));
        let mut file = File::create(&file_path).await?;

        // 读取 multipart field 数据
        let bytes = field.bytes().await.map_err(|e| AppError::Multipart(e.to_string()))?;
        size += bytes.len();
        if size > 50 * 1024 * 1024 {
            // 删除临时文件
            let _ = tokio::fs::remove_file(&file_path).await;
            return Err(AppError::FileTooLarge);
        }
        hasher.update(&bytes);
        file.write_all(&bytes).await?;

        file.flush().await?;

        let hash = hex::encode(hasher.finalize());

        info!("Uploaded file: {} (size: {} bytes, hash: {})", file_name, size, hash);

        // 返回 JSON（url 可用于事件 tag）
        return Ok(Json(json!({
            "url": format!("/blobs/{}.{}", uuid, file_ext),
            "hash": hash,
            "mime": content_type,
            "size": size
        })));
    }

    Err(AppError::Internal("no file found".to_string()))
}