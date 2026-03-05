use async_trait::async_trait;
use serde_json::Value;
use futures::StreamExt;
use libp2p::{
    gossipsub::{Gossipsub, GossipsubConfigBuilder, GossipsubEvent, IdentTopic, MessageAuthenticity},
    identity::Keypair,
    kad::{store::MemoryStore, Kademlia, KademliaConfig, KademliaEvent},
    multiaddr::Protocol,
    swarm::{Swarm, SwarmBuilder, SwarmEvent},
    tcp::TcpConfig,
    Multiaddr,
    PeerId,
    Transport,
};

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tracing::{error, info};

use crate::filter::Filter;

// MeshProxy trait (from previous)
#[async_trait]
pub trait MeshProxy: Send + Sync {
    async fn forward_event(&self, event: Value) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn query_remote(
        &self, filter: Filter,
    ) -> Result<Vec<Value>, Box<dyn std::error::Error + Send + Sync>>;
    async fn announce_online(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn health_check(&self) -> bool;
}

// Custom behaviour combining Kademlia (discovery + DHT) and Gossipsub (forwarding)
#[derive(NetworkBehaviour)]
struct CustomBehaviour {
    kademlia: Kademlia<MemoryStore>,
    gossipsub: Gossipsub,
}

// Libp2p based implementation
pub struct Libp2pMeshProxy {
    swarm: Arc<Swarm<CustomBehaviour>>,
    event_tx: Sender<Value>,
    query_tx: Sender<(Filter, oneshot::Sender<Vec<Value>>)>,
    shutdown_tx: Sender<()>,
}

impl Libp2pMeshProxy {
    pub fn new(local_key: Keypair, listen_addr: Multiaddr, bootnodes: Vec<Multiaddr>) -> Self {
        let local_peer_id = PeerId::from(local_key.public());

        let transport = TcpConfig::new().into();

        let mut kademlia_config = KademliaConfig::default();
        kademlia_config.set_kbucket_inserts(true);
        let kademlia = Kademlia::with_config(
            local_peer_id,
            MemoryStore::new(local_peer_id),
            kademlia_config,
        );

        let gossipsub_config = GossipsubConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(10))
            .validation_mode(ValidationMode::Strict)
            .build()
            .expect("Valid config");
        let gossipsub = Gossipsub::new(MessageAuthenticity::Signed(local_key), gossipsub_config)
            .expect("Gossipsub initialization");

        let behaviour = CustomBehaviour { kademlia, gossipsub };

        let mut swarm = SwarmBuilder::new(transport, behaviour, local_peer_id)
            .build();

        swarm.listen_on(listen_addr).expect("Listen failed");

        for bootnode in bootnodes {
            let peer_id = bootnode.iter().find_map(|p| if let Protocol::P2p(hash) = p { Some(PeerId::from_multihash(hash).ok()?) } else { None });
            if let Some(peer_id) = peer_id {
                swarm.behaviour_mut().kademlia.add_address(&peer_id, bootnode);
            }
        }

        let (event_tx, event_rx) = mpsc::channel(32);
        let (query_tx, query_rx) = mpsc::channel(32);
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

        let swarm_arc = Arc::new(swarm);

        // Spawn swarm loop task
        tokio::spawn(run_swarm(swarm_arc.clone(), event_rx, query_rx, shutdown_rx));

        Self {
            swarm: swarm_arc,
            event_tx,
            query_tx,
            shutdown_tx,
        }
    }
}

async fn run_swarm(
    mut swarm: Swarm<CustomBehaviour>,
    mut event_rx: Receiver<Value>,
    mut query_rx: Receiver<(Filter, oneshot::Sender<Vec<Value>>)>,
    mut shutdown_rx: Receiver<()>,
) {
    let im_topic = IdentTopic::new("im-events");

    swarm.behaviour_mut().gossipsub.subscribe(&im_topic).expect("Subscribe failed");

    loop {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                // Forward event via Gossipsub
                let topic = IdentTopic::new("im-events");
                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, serde_json::to_vec(&event).unwrap()) {
                    error!("Gossipsub publish failed: {:?}", e);
                }
            }

            Some((filter, tx)) = query_rx.recv() => {
                // Query remote via Kademlia DHT
                // Here we assume filter can be serialized to a key, e.g., hash of filter
                let key = hash_filter(&filter);
                swarm.behaviour_mut().kademlia.get_record(&key);
                // Wait for event in swarm loop and send back (simplified, need to handle in event loop)
                // For now, dummy return empty
                let _ = tx.send(vec![]);
            }

            Some(event) = swarm.select_next_some() => {
                match event {
                    SwarmEvent::Behaviour(CustomBehaviourEvent::Kademlia(event)) => {
                        match event {
                            KademliaEvent::OutboundQueryProgressed { result, .. } => {
                                // Handle query results (e.g., get backup count)
                                info!("Kademlia query result: {:?}", result);
                            }
                            KademliaEvent::RoutingUpdated { peer, .. } => {
                                info!("Peer discovered: {:?}", peer);
                            }
                            _ => {}
                        }
                    }
                    SwarmEvent::Behaviour(CustomBehaviourEvent::Gossipsub(event)) => {
                        if let GossipsubEvent::Message { message, .. } = event {
                            // Handle received forwarded event
                            let event: Value = serde_json::from_slice(&message.data).unwrap_or_default();
                            info!("Received forwarded event: {:?}", event);
                            // Process or broadcast locally
                        }
                    }
                    SwarmEvent::NewListenAddr { address, .. } => {
                        info!("Listening on {:?}", address);
                    }
                    _ => {}
                }
            }

            Some(_) = shutdown_rx.recv() => {
                break;
            }
        }
    }
}

/// Hash filter for DHT key (dummy implementation, replace with proper serialization)
fn hash_filter(filter: &Filter) -> libp2p::kad::record::Key {
    let serialized = serde_json::to_string(filter).unwrap_or_default();
    libp2p::kad::record::Key::new(&serialized)
}

#[async_trait]
impl MeshProxy for Libp2pMeshProxy {
    async fn forward_event(&self, event: Value) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.event_tx.send(event).await.map_err(|_| "Send failed".into())
    }

    async fn query_remote(
        &self,
        filter: Filter,
    ) -> Result<Vec<Value>, Box<dyn std::error::Error + Send + Sync>> {
        let (tx, rx) = oneshot::channel();
        self.query_tx.send((filter, tx)).await.map_err(|_| "Send failed".into())?;
        rx.await.map_err(|_| "Recv failed".into())
    }

    async fn announce_online(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Put backup count to DHT (e.g., key = "backup_count:my_peer_id", value = "2")
        let peer_id = self.swarm.local_peer_id().to_string();
        let key = libp2p::kad::record::Key::new(&format!("backup_count:{}", peer_id));
        let value = b"2".to_vec();
        self.swarm.behaviour_mut().kademlia.put_record(libp2p::kad::Record { key, value, publisher: None, expires: None }, libp2p::kad::Quorum::One)
            .expect("Put failed");
        Ok(())
    }

    async fn health_check(&self) -> bool {
        self.swarm.behaviour_mut().kademlia.kbuckets().count() > 0  // Check if has peers
    }
}

impl Drop for Libp2pMeshProxy {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.try_send(());
    }
}

/// P2P mesh 网络转发代理 trait
/// 所有方法都是异步的，返回 Result 以便错误处理
#[async_trait]
pub trait MeshProxy: Send + Sync + 'static {
    /// 将事件转发给 mesh 网络中的其他 relay
    async fn forward_event(&self, event: Value) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// 向 mesh 网络查询匹配 filter 的事件（用于 REQ 时本地无数据）
    async fn query_remote(
        &self,
        filter: crate::filter::Filter,
    ) -> Result<Vec<Value>, Box<dyn std::error::Error + Send + Sync>>;

    /// 通知 mesh 网络本 relay 已上线（可选，用于发现）
    async fn announce_online(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// 检查 mesh 网络是否健康（用于监控）
    async fn health_check(&self) -> bool {
        true
    }
}

/// 一个简单的 Dummy 实现（开发阶段占位）
pub struct DummyMeshProxy;

#[async_trait]
impl MeshProxy for DummyMeshProxy {
    async fn forward_event(&self, event: Value) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("DummyMeshProxy: forwarding event → {}", event["id"].as_str().unwrap_or("unknown"));
        // 模拟网络延迟
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        Ok(())
    }

    async fn query_remote(
        &self,
        filter: crate::filter::Filter,
    ) -> Result<Vec<Value>, Box<dyn std::error::Error + Send + Sync>> {
        info!("DummyMeshProxy: querying remote with filter: {:?}", filter);
        // 模拟返回空结果
        Ok(vec![])
    }

    async fn announce_online(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("DummyMeshProxy: announcing relay online");
        Ok(())
    }

    async fn health_check(&self) -> bool {
        info!("DummyMeshProxy: health check OK");
        true
    }
}