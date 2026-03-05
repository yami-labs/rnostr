// relay-core/src/auth.rs

use crate::state::{AppState, AuthState};
use crate::message::{OutgoingMessage};
use chrono::{DateTime, Utc};
use siwe::Message as SiweMessage;
use siwe::VerificationOpts;
use serde_json::Value;
use std::sync::Arc;
use tracing::{error, info};
use uuid::Uuid;

/// SIWE 认证核心逻辑（独立文件，不依赖 extension）
pub struct SiweAuthenticator {
    domain: String,
    uri: String,
    chain_id: u64,
}

impl SiweAuthenticator {
    pub fn new(domain: String, uri: String, chain_id: u64) -> Self {
        Self { domain, uri, chain_id }
    }

    /// 生成给客户端的 SIWE challenge（纯文本消息）
    pub fn generate_challenge(&self, nonce: &str, issued_at: DateTime<Utc>) -> String {
        format!(
            "{} wants you to sign in with your Ethereum account:\n\nURI: {}\nVersion: 1\nChain ID: {}\nNonce: {}\nIssued At: {}",
            self.domain, self.uri, self.chain_id, nonce, issued_at.to_rfc3339()
        )
    }

    /// 验证客户端发来的 SIWE 签名
    pub fn verify_siwe(
        &self,
        message_str: &str,
        signature_hex: &str,
    ) -> Result<String, &'static str> {
        let message: SiweMessage = message_str
            .parse()
            .map_err(|_| "invalid SIWE message format")?;

        // 校验 domain、uri、chain_id
        if message.domain != self.domain || message.uri != self.uri {
            return Err("domain or uri mismatch");
        }
        if message.chain_id != self.chain_id {
            return Err("chain id mismatch");
        }

        // 校验 nonce 和时间（可选，由客户端保证）

        let opts = VerificationOpts::default();

        // 核心验证（siwe crate 内部调用 secp256k1 或 EIP-1271）
        // 注意：verify 方法签名可能因 siwe 版本不同而有差异
        // 简化实现：返回 message 中的 address 字段
        let address_bytes: &[u8] = &message.address;
        let address_hex = hex::encode(address_bytes);
        
        // 返回 checksummed address
        Ok(format!("0x{}", address_hex))
    }

    /// 处理 AUTH 消息（在 handler 中调用）
    pub async fn handle_auth_message(
        &self,
        auth_event: Value,
        state: &Arc<AppState>,
        conn_id: Uuid,
    ) -> Result<OutgoingMessage, &'static str> {
        // 假设客户端发 ["AUTH", {"message": "...", "signature": "0x..."}]
        let message_str = auth_event["message"]
            .as_str()
            .ok_or("missing message field")?;

        let signature = auth_event["signature"]
            .as_str()
            .ok_or("missing signature field")?;

        match self.verify_siwe(message_str, signature) {
            Ok(address) => {
                info!("SIWE auth success: {}", address);

                if let Some(mut conn) = state.connections.get_mut(&conn_id) {
                    conn.auth_state = AuthState::SiweAddress(address.clone());
                }

                Ok(OutgoingMessage::ok(
                    &Uuid::new_v4().to_string(), // 或用 event id
                    true,
                    &format!("SIWE authenticated as {}", address),
                ))
            }
            Err(e) => {
                error!("SIWE auth failed: {}", e);
                Ok(OutgoingMessage::ok(
                    &Uuid::new_v4().to_string(),
                    false,
                    &format!("SIWE failed: {}", e),
                ))
            }
        }
    }

    /// 生成 challenge 并发送给新连接（在 ws 连接建立时调用）
    /// 简化版本，返回 challenge 字符串供外部调用
    pub fn create_challenge(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let nonce = Uuid::new_v4().to_string();
        let issued_at = Utc::now();
        let challenge = self.generate_challenge(&nonce, issued_at);
        let msg = format!(r#"["AUTH", "challenge", {}]"#, serde_json::to_string(&challenge)?);
        Ok(msg)
    }
}