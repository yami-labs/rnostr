// relay-core/src/config.rs（扩展版）

use config::{Config, File, FileFormat};
use notify::{recommended_watcher, RecursiveMode, Watcher};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::sync::watch;
use tracing::{error, info};

#[derive(Debug, Clone, Deserialize)]
pub struct RelayConfig {
    pub listen_addr: String,
    pub lmdb_path: String,
    pub blob_dir: String,
    pub siwe: SiweConfig,
    pub gc: GcConfig,
    pub connection: ConnectionConfig,
    
    // metrics 配置（运行时开关）
    pub metrics: MetricsConfig,
    
    // 新增：TLS 配置
    pub tls: TlsConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TlsConfig {
    pub enabled: bool,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub endpoint: String,  // 如 "/metrics"
}

#[derive(Debug, Clone, Deserialize)]
pub struct SiweConfig {
    pub domain: String,
    pub uri: String,
    pub chain_id: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GcConfig {
    pub enabled: bool,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionConfig {
    pub max_connections: usize,
    pub max_subscriptions_per_conn: usize,
    pub event_ttl_seconds: u64,
}

pub struct ConfigManager {
    pub config: Arc<RwLock<RelayConfig>>,
    pub reload_tx: watch::Sender<()>,
}

impl ConfigManager {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let path = path.into();

        let cfg = Config::builder()
            .add_source(File::from(path.clone()).required(true).format(FileFormat::Toml))
            .build()?
            .try_deserialize::<RelayConfig>()?;

        let (tx, _) = watch::channel(());

        let manager = Self {
            config: Arc::new(RwLock::new(cfg)),
            reload_tx: tx,
        };

        manager.watch_config_file(path)?;

        Ok(manager)
    }

    fn watch_config_file(&self, path: PathBuf) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let config = self.config.clone();
        let reload_tx = self.reload_tx.clone();
        let path_clone = path.clone();

        let mut watcher = recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                if event.paths.iter().any(|p| p == &path_clone) {
                    info!("Config changed, reloading...");
                    if let Err(e) = Self::reload_config(&config, &path_clone) {
                        error!("Reload failed: {}", e);
                    } else {
                        let _ = reload_tx.send(());
                    }
                }
            }
        })?;

        watcher.watch(&path, RecursiveMode::NonRecursive)?;
        info!("Watching config: {:?}", path);
        Ok(())
    }

    fn reload_config(config: &Arc<RwLock<RelayConfig>>, path: &PathBuf) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let new_cfg = Config::builder()
            .add_source(File::from(path.clone()).format(FileFormat::Toml))
            .build()?
            .try_deserialize::<RelayConfig>()?;

        let mut guard = config.write().unwrap();
        *guard = new_cfg;
        info!("Config reloaded");
        Ok(())
    }

    pub fn get(&self) -> RelayConfig {
        self.config.read().unwrap().clone()
    }
}