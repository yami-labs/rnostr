Relay-IM-Core Project Documentation1. Project Overview and Design IntentProject NameRelay-IM-CoreProject DescriptionRelay-IM-Core is a minimalist, decentralized instant messaging (IM) relay server built on top of the Nostr protocol. It focuses on private direct messages (DM) and group chats (rooms), prioritizing privacy, minimal persistence, and decentralization. The project forks from rnostr but heavily customizes it to remove public feed features, simplifying it into an IM-oriented system.Design IntentThe core intent is to create a lightweight relay for secure, real-time IM communication without the overhead of full Nostr social features. Key principles:

- Privacy-First: Use E2E encryption for DMs (NIP-17) and simplified shared-key encryption for rooms. Minimize metadata exposure and relay-side data retention.
- Minimal Persistence: Messages and files expire automatically (default 1 year, min 6 months). Clients cache history; relay only provides temporary storage/forwarding.
- Decentralization via P2P: Relays discover each other and forward messages/queries via libp2p mesh, with double backup for resilience.
- Web3 Integration: Authentication via SIWE (Sign-In with Ethereum), tying user identity to Ethereum addresses for sybil resistance.
- Simplicity and Efficiency: Target 1000 concurrent users on 4C8G hardware. No public feeds, searches, or complex NIPs—only DM/room essentials.
- Target Use Cases: Private chats for DAOs, small teams, or communities needing secure, decentralized messaging without centralized servers.
- Non-Goals: Full Nostr compatibility, large-scale social networking, permanent storage, or advanced features like VOIP (future extension).

The project assumes good-faith users and focuses on MVP usability, with extensibility for future plugins (e.g., metrics via features).2. ArchitectureHigh-Level ArchitectureThe system is a single-binary Rust application using axum for HTTP/WS, libp2p for P2P, and LMDB for storage. It's structured as a library-like core with minimal CLI.

- Frontend/Client: Not included (assume external PWA or app using nostr-tools). Clients connect via WS, authenticate with SIWE, send/receive events.
- Relay Core: Handles WS connections, message routing, storage, and P2P.
- P2P Mesh: Discovery, forwarding, and backup between relays.

Data Flow:

1.  Client connects WS → receives SIWE challenge → signs and sends AUTH.
2.  Client sends EVENT (DM/room message) → relay validates (auth, kind, expire) → stores in LMDB → broadcasts locally or forwards via P2P.
3.  Client sends REQ (query history) → relay queries LMDB → if no results, proxies via P2P query → returns events.
4.  GC task periodically cleans expired data.

Components and Modules

- src/main.rs: Entry point, loads config, initializes state, starts axum server (WS + HTTP endpoints like /upload, /metrics, /health), spawns GC/mesh tasks.
- src/state.rs: AppState (global shared state: connections DashMap, broadcast channel, config RwLock, mesh_proxy, room_index); ConnectionState (per-conn: auth, subscriptions).
- src/ws.rs: WS upgrade handler, connection loop (challenge send, message recv/dispatch, broadcast push, cleanup).
- src/handler.rs: Message handlers (handle_event, handle_req, etc.), with IM filters and P2P fallback.
- src/message.rs: Incoming/OutgoingMessage structs, parse/serialize.
- src/auth.rs: SIWE authenticator (challenge generate, verify).
- src/filter.rs: Filter struct, is_im_compatible/matches methods.
- src/storage.rs: ImStorage wrapper over db/ (LMDB ops: save/query/gc).
- src/db/: Built-in LMDB (lmdb.rs, scanner.rs, mod.rs for exports).
- src/gc.rs: Garbage collection for events/files (timed task).
- src/config.rs: ConfigManager (load/hot-reload config.toml).
- src/mesh.rs: Libp2pMesh (discovery, double backup, forward/query).
- src/upload.rs: File upload handler (50MB limit, MIME/ext checks, store to blobs/).
- extensions/metrics/: Optional metrics plugin (feature-gated, Prometheus counters/gauges).

Technical Stack

- Web/WS Server: axum (HTTP routes, WS upgrade)
- P2P: libp2p (Kademlia discovery, Gossipsub forwarding)
- Storage: LMDB (embedded KV, fast scans)
- Auth: siwe + alloy (Ethereum signature verify)
- Encryption: Client-side (NIP-44 for DM, AES-GCM for rooms)
- Config: toml + notify (hot-reload)
- Logging: tracing
- Metrics: metrics crate (optional feature)
- TLS: rustls (config-enabled)
- Other: uuid, serde_json, dashmap, parking_lot, chrono

Security Considerations

- E2E Encryption: Client responsibility (relay blind)
- Auth: SIWE only, no fallback
- Rate Limit: Built-in (conn config)
- Data Retention: Strict expire/GC
- P2P: libp2p noise auth, but no end-to-end peer verify (future add)

3\. Future Update Plan (Roadmap)Short-Term (Next 1–3 Months: MVP Stabilization)

- Q1 2026: Basic testing suite (unit + e2e WS tests)
- Add rate limiting extension (feature-gated)
- Client demo (simple PWA for DM/room)
- Performance benchmark (1000 conn on 4C8G)
- Docker image + deployment guide

Medium-Term (3–6 Months: Feature Expansion)

- Q2 2026: Room E2E encryption (shared key via NIP-29 events)
- Offline message delivery (queue + TTL)
- Multi-relay sync (full P2P query/forward)
- VOIP signaling (NIP for WebRTC setup)
- Mobile push notifications (via relay or external service)

Long-Term (6–12 Months: Ecosystem & Scalability)

- Q3 2026: Extract storage as separate crate (nostr-im-kv)
- Federation support (relay clusters)
- ZK proofs for privacy (optional auth)
- Android/iOS client integration
- Community contributions (open source on GitHub)

Risk & Contingency

- If libp2p overhead高，fallback to simple WebSocket relay peering
- Monitor LMDB scale; if >1M events, add sharding
- Security audit before public release

This roadmap is flexible, based on user feedback and resource availability.