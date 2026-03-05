// relay-core/src/mesh.rs

use crate::error::Error as AppError;
use crate::filter::Filter;
use async_trait::async_trait;
use futures::prelude::*;
use libp2p::{
    gossipsub::{self, IdentTopic, MessageAuthenticity},
    identify,
    kad::{
        self, store::MemoryStore, GetRecordOk, PeerRecord, QueryResult, RecordKey,
    },
    mdns,
    noise,
    ping,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId, SwarmBuilder,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use tracing::{error, info, warn};

// ============================================
// 常量定义
// ============================================
const TOPIC_IM_EVENTS: &str = "im-relay-events";
const TOPIC_BACKUP_REQUEST: &str = "backup-request";
const TOPIC_QUERY_REQUEST: &str = "query-request";
const TOPIC_QUERY_RESPONSE: &str = "query-response";

// ============================================
// Trait 定义
// ============================================
#[async_trait]
pub trait MeshProxy: Send + Sync {
    async fn forward_event(&self, event: Value) -> Result<(), AppError>;
    async fn query_remote(&self, filter: Filter) -> Result<Vec<Value>, AppError>;
    async fn announce_online(&self) -> Result<(), AppError>;
    fn health_check(&self) -> bool;
}

// ============================================
// 内部类型
// ============================================
#[derive(Debug)]
enum QueryType {
    GetBackup,
    RemoteFilter(Filter),
}

#[derive(Debug)]
enum MyEvent {
    Gossipsub(gossipsub::Event),
    Kademlia(kad::Event),
    Mdns(mdns::Event),
    Identify(identify::Event),
    Ping(ping::Event),
}

impl From<gossipsub::Event> for MyEvent {
    fn from(event: gossipsub::Event) -> Self {
        MyEvent::Gossipsub(event)
    }
}

impl From<kad::Event> for MyEvent {
    fn from(event: kad::Event) -> Self {
        MyEvent::Kademlia(event)
    }
}

impl From<mdns::Event> for MyEvent {
    fn from(event: mdns::Event) -> Self {
        MyEvent::Mdns(event)
    }
}

impl From<identify::Event> for MyEvent {
    fn from(event: identify::Event) -> Self {
        MyEvent::Identify(event)
    }
}

impl From<ping::Event> for MyEvent {
    fn from(event: ping::Event) -> Self {
        MyEvent::Ping(event)
    }
}

// ============================================
// NetworkBehaviour 定义
// ============================================
#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "MyEvent")]
struct MyBehaviour {
    gossipsub: gossipsub::Behaviour,
    kademlia: kad::Behaviour<MemoryStore>,
    mdns: mdns::tokio::Behaviour,
    identify: identify::Behaviour,
    ping: ping::Behaviour,
}

// ============================================
// Libp2pMesh 实现
// ============================================
pub struct Libp2pMesh {
    swarm: libp2p::swarm::Swarm<MyBehaviour>,
    local_peer_id: PeerId,
    topic_im_events: IdentTopic,
    topic_backup_request: IdentTopic,
    topic_query_request: IdentTopic,
    topic_query_response: IdentTopic,
    backup_key: RecordKey,
    pending_queries: HashMap<kad::QueryId, QueryType>,
    known_peers: HashSet<PeerId>,
}

impl Libp2pMesh {
    pub fn new(
        listen_addr: Multiaddr,
        bootstrap_peers: Vec<Multiaddr>,
    ) -> Result<Self, AppError> {
        let local_key = libp2p::identity::Keypair::generate_ed25519();
        let local_peer_id = PeerId::from(local_key.public());

        let mut swarm = SwarmBuilder::with_existing_identity(local_key)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .map_err(|e| AppError::Network(format!("TCP transport error: {:?}", e)))?
            .with_behaviour(|key| {
                let local_peer_id = PeerId::from(key.public());
                
                let gossipsub = gossipsub::Behaviour::new(
                    MessageAuthenticity::Signed(key.clone()),
                    gossipsub::Config::default(),
                ).expect("gossipsub init failed");

                let mut kademlia = kad::Behaviour::new(
                    local_peer_id,
                    MemoryStore::new(local_peer_id),
                );

                // 添加 bootstrap peers
                for addr in &bootstrap_peers {
                    if let Some(peer_id) = addr.iter().find_map(|p| {
                        if let libp2p::multiaddr::Protocol::P2p(hash) = p {
                            Some(PeerId::from(hash))
                        } else {
                            None
                        }
                    }) {
                        kademlia.add_address(&peer_id, addr.clone());
                    }
                }

                let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)
                    .expect("mdns init failed");
                let identify = identify::Behaviour::new(identify::Config::new(
                    "/im-relay/1.0.0".into(),
                    key.public(),
                ));
                let ping = ping::Behaviour::new(ping::Config::new());

                MyBehaviour {
                    gossipsub,
                    kademlia,
                    mdns,
                    identify,
                    ping,
                }
            })
            .map_err(|e| AppError::Network(format!("behaviour error: {:?}", e)))?
            .build();

        let local_peer_id = *swarm.local_peer_id();

        // 创建 topics
        let topic_im_events = IdentTopic::new(TOPIC_IM_EVENTS);
        let topic_backup_request = IdentTopic::new(TOPIC_BACKUP_REQUEST);
        let topic_query_request = IdentTopic::new(TOPIC_QUERY_REQUEST);
        let topic_query_response = IdentTopic::new(TOPIC_QUERY_RESPONSE);

        // 订阅所有 topics
        let behaviour = swarm.behaviour_mut();
        behaviour.gossipsub.subscribe(&topic_im_events)
            .map_err(|e| AppError::Gossipsub(format!("subscribe failed: {:?}", e)))?;
        behaviour.gossipsub.subscribe(&topic_backup_request)
            .map_err(|e| AppError::Gossipsub(format!("subscribe failed: {:?}", e)))?;
        behaviour.gossipsub.subscribe(&topic_query_request)
            .map_err(|e| AppError::Gossipsub(format!("subscribe failed: {:?}", e)))?;
        behaviour.gossipsub.subscribe(&topic_query_response)
            .map_err(|e| AppError::Gossipsub(format!("subscribe failed: {:?}", e)))?;

        // 启动监听
        swarm.listen_on(listen_addr)
            .map_err(|e| AppError::Network(format!("listen failed: {:?}", e)))?;

        let backup_key = RecordKey::new(&format!("backup_count:{}", local_peer_id));

        Ok(Self {
            swarm,
            local_peer_id,
            topic_im_events,
            topic_backup_request,
            topic_query_request,
            topic_query_response,
            backup_key,
            pending_queries: HashMap::new(),
            known_peers: HashSet::new(),
        })
    }

    pub async fn run(&mut self) {
        loop {
            tokio::select! {
                event = self.swarm.select_next_some() => {
                    self.handle_event(event).await;
                }
            }
        }
    }

    async fn handle_event(&mut self, event: SwarmEvent<MyEvent>) {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                info!("Listening on {}", address);
            }
            
            SwarmEvent::Behaviour(MyEvent::Gossipsub(gossipsub::Event::Message {
                propagation_source,
                message,
                ..
            })) => {
                match serde_json::from_slice::<Value>(&message.data) {
                    Ok(event) => {
                        info!("Received from {}: {:?}", propagation_source, event);
                        // TODO: 根据 event["type"] 分发处理
                    }
                    Err(e) => {
                        warn!("Failed to parse message: {}", e);
                    }
                }
            }

            SwarmEvent::Behaviour(MyEvent::Kademlia(kad::Event::OutboundQueryProgressed {
                id,
                result: QueryResult::GetRecord(result),
                ..
            })) => {
                match result {
                    Ok(GetRecordOk::FoundRecord(PeerRecord { record, .. })) => {
                        info!("Found record: {:?}", record);
                        if let Some(query_type) = self.pending_queries.remove(&id) {
                            match query_type {
                                QueryType::GetBackup => {
                                    info!("Backup count: {:?}", record.value);
                                }
                                QueryType::RemoteFilter(filter) => {
                                    info!("Query result for {:?}: {:?}", filter, record.value);
                                }
                            }
                        }
                    }
                    Ok(GetRecordOk::FinishedWithNoAdditionalRecord { .. }) => {
                        warn!("Record not found for query {}", id);
                        self.pending_queries.remove(&id);
                    }
                    Err(e) => {
                        error!("Query {} failed: {:?}", id, e);
                        self.pending_queries.remove(&id);
                    }
                }
            }

            SwarmEvent::Behaviour(MyEvent::Kademlia(kad::Event::RoutingUpdated {
                peer,
                is_new_peer,
                ..
            })) => {
                if is_new_peer {
                    info!("New peer discovered: {}", peer);
                    self.known_peers.insert(peer);
                }
            }

            SwarmEvent::Behaviour(MyEvent::Mdns(mdns::Event::Discovered(list))) => {
                for (peer_id, addr) in list {
                    info!("mDNS discovered: {} at {}", peer_id, addr);
                    self.swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                    self.known_peers.insert(peer_id);
                }
            }

            SwarmEvent::Behaviour(MyEvent::Mdns(mdns::Event::Expired(list))) => {
                for (peer_id, addr) in list {
                    info!("mDNS expired: {} at {}", peer_id, addr);
                    self.swarm.behaviour_mut().kademlia.remove_address(&peer_id, &addr);
                    self.known_peers.remove(&peer_id);
                }
            }

            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                info!("Connected to {}", peer_id);
                self.known_peers.insert(peer_id);
            }

            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                info!("Disconnected from {}", peer_id);
                self.known_peers.remove(&peer_id);
            }

            _ => {}
        }
    }

    fn find_backup_peers(&self) -> Vec<PeerId> {
        self.known_peers.iter().take(2).cloned().collect()
    }

    async fn send_backup_request(
        &mut self,
        peers: Vec<PeerId>,
    ) -> Result<(), AppError> {
        if peers.is_empty() {
            return Err(AppError::PeerNotFound("no peers available for backup".to_string()));
        }
        for peer_id in peers {
            let request = json!({
                "type": "backup_request",
                "from": self.local_peer_id.to_string(),
                "target": peer_id.to_string(),
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });
            let data = serde_json::to_vec(&request)?;
            self.swarm
                .behaviour_mut()
                .gossipsub
                .publish(self.topic_backup_request.clone(), data)
                .map_err(|e| AppError::Gossipsub(format!("publish failed: {:?}", e)))?;
            info!("Sent backup request to {}", peer_id);
        }
        Ok(())
    }
}

// TODO: Libp2pMesh 的 MeshProxy 实现需要重构以支持 &self 而不是 &mut self
// 暂时注释掉，使用 DummyMesh 进行开发

/*
#[async_trait]
impl MeshProxy for Libp2pMesh {
    async fn forward_event(
        &mut self,
        event: Value,
    ) -> Result<(), AppError> {
        let data = serde_json::to_vec(&event)?;
        self.swarm
            .behaviour_mut()
            .gossipsub
            .publish(self.topic_im_events.clone(), data)
            .map_err(|e| AppError::Gossipsub(format!("publish failed: {:?}", e)))?;
        Ok(())
    }

    async fn query_remote(
        &mut self,
        filter: Filter,
    ) -> Result<Vec<Value>, AppError> {
        let request = json!({
            "type": "query_request",
            "filter": filter,
            "request_id": Uuid::new_v4().to_string(),
            "from": self.local_peer_id.to_string(),
        });
        let data = serde_json::to_vec(&request)?;
        self.swarm
            .behaviour_mut()
            .gossipsub
            .publish(self.topic_query_request.clone(), data)
            .map_err(|e| AppError::Gossipsub(format!("publish failed: {:?}", e)))?;
        
        // TODO: 实际应该使用 channel 等待响应
        tokio::time::sleep(Duration::from_secs(5)).await;
        Ok(vec![])
    }

    async fn announce_online(
        &mut self,
    ) -> Result<(), AppError> {
        // 1. 发布备份计数到 DHT
        let record = Record {
            key: self.backup_key.clone(),
            value: b"2".to_vec(),
            publisher: Some(self.local_peer_id),
            expires: None,
        };
        self.swarm
            .behaviour_mut()
            .kademlia
            .put_record(record, Quorum::One)
            .map_err(|e| AppError::Kademlia(format!("put_record failed: {:?}", e)))?;

        // 2. 寻找备份节点
        let peers = self.find_backup_peers();
        if peers.len() < 2 {
            warn!("Not enough peers for double backup: {}/2", peers.len());
            return Err(AppError::InsufficientPeers);
        } else {
            self.send_backup_request(peers).await?;
        }

        Ok(())
    }

    fn health_check(&self) -> bool {
        !self.known_peers.is_empty()
    }
}
*/

// ============================================
// DummyMesh - 用于开发和测试
// ============================================
#[derive(Clone)]
pub struct DummyMesh;

#[async_trait]
impl MeshProxy for DummyMesh {
    async fn forward_event(&self, _event: Value) -> Result<(), AppError> {
        // 不转发任何事件
        Ok(())
    }

    async fn query_remote(&self, _filter: Filter) -> Result<Vec<Value>, AppError> {
        // 返回空结果
        Ok(Vec::new())
    }

    async fn announce_online(&self) -> Result<(), AppError> {
        // 无操作
        Ok(())
    }

    fn health_check(&self) -> bool {
        // 总是健康的
        true
    }
}