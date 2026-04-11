//! P2P network manager — wraps libp2p swarm and exposes high-level events.

use borsh::BorshDeserialize;
use libp2p::{
    gossipsub, identity, mdns, noise,
    request_response::{self},
    swarm::SwarmEvent,
    tcp, yamux, Multiaddr, PeerId, Swarm, SwarmBuilder,
};
use std::collections::HashSet;
use std::path::Path;
use tokio::sync::mpsc;
use tracing;

use crate::behaviour::{ClawBehaviour, ClawBehaviourEvent};
use crate::protocol::{self, *};

/// Events emitted by the P2P network to the chain engine.
#[derive(Debug)]
pub enum NetworkEvent {
    /// Received a new transaction from the network.
    NewTx(claw_types::transaction::Transaction),
    /// Received a new block from the network.
    NewBlock(claw_types::block::Block),
    /// Received a BFT vote from a validator.
    Vote(BlockVote),
    /// Received a miner checkin witness (V3).
    MinerCheckin(claw_types::state::MinerCheckinWitness),
    /// A sync request from a peer (includes response channel for replying).
    SyncRequest {
        peer: PeerId,
        request_id: request_response::InboundRequestId,
        request: SyncRequest,
        channel: request_response::ResponseChannel<Vec<u8>>,
    },
    /// A sync response from a peer.
    SyncResponse {
        peer: PeerId,
        response: SyncResponse,
    },
    /// A new peer connected.
    PeerConnected(PeerId),
    /// A peer disconnected.
    PeerDisconnected(PeerId),
}

/// Commands that the chain engine can send back to the P2P network.
#[derive(Debug)]
pub enum P2pCommand {
    /// Send a sync request to a specific peer.
    SendSyncRequest { peer: PeerId, request: SyncRequest },
    /// Send a sync response via the provided channel.
    SendSyncResponse {
        channel: request_response::ResponseChannel<Vec<u8>>,
        response: SyncResponse,
    },
    /// Broadcast a newly produced block to the network.
    BroadcastBlock(claw_types::block::Block),
    /// Broadcast a BFT vote to the network.
    BroadcastVote(BlockVote),
    /// Broadcast a transaction to the network (for non-validator nodes to propagate txs).
    BroadcastTx(claw_types::Transaction),
    /// Broadcast a miner checkin witness (V3).
    BroadcastCheckin(claw_types::state::MinerCheckinWitness),
}

/// P2P Network handle for interacting with the swarm.
pub struct P2pNetwork {
    swarm: Swarm<ClawBehaviour>,
    event_tx: mpsc::UnboundedSender<NetworkEvent>,
    command_rx: mpsc::UnboundedReceiver<P2pCommand>,
    peers: HashSet<PeerId>,
    /// Peers that failed protocol negotiation (e.g. different chain_id).
    /// Prevents mDNS from repeatedly re-adding incompatible peers.
    incompatible_peers: HashSet<PeerId>,
    tx_topic: gossipsub::IdentTopic,
    block_topic: gossipsub::IdentTopic,
    vote_topic: gossipsub::IdentTopic,
    checkin_topic: gossipsub::IdentTopic,
    /// Bootstrap addresses for periodic reconnection.
    bootstrap_addrs: Vec<Multiaddr>,
}

/// Load an existing P2P keypair from disk, or generate a new one and save it.
///
/// The keypair is stored as protobuf-encoded bytes at `<data_dir>/p2p_key`.
/// File permissions are set to 0600 (owner read/write only) on Unix systems.
fn load_or_generate_keypair(data_dir: &Path) -> Result<identity::Keypair, Box<dyn std::error::Error>> {
    let key_path = data_dir.join("p2p_key");
    if key_path.exists() {
        let bytes = std::fs::read(&key_path)
            .map_err(|e| format!("failed to read P2P key from {}: {e}", key_path.display()))?;
        let keypair = identity::Keypair::from_protobuf_encoding(&bytes)
            .map_err(|e| format!("failed to decode P2P key from {}: {e}", key_path.display()))?;
        tracing::info!(path = %key_path.display(), "Loaded existing P2P keypair");
        Ok(keypair)
    } else {
        let keypair = identity::Keypair::generate_ed25519();
        let bytes = keypair.to_protobuf_encoding()
            .map_err(|e| format!("failed to encode P2P key: {e}"))?;

        // Ensure parent directory exists
        if let Some(parent) = key_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&key_path, &bytes)
            .map_err(|e| format!("failed to write P2P key to {}: {e}", key_path.display()))?;

        // Set restrictive file permissions (owner read/write only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
        }

        tracing::info!(path = %key_path.display(), "Generated and saved new P2P keypair");
        Ok(keypair)
    }
}

impl P2pNetwork {
    /// Create a new P2P network with chain_id-scoped protocols.
    /// Returns (network, event_receiver, command_sender).
    ///
    /// The `data_dir` is used to persist the P2P keypair so the peer ID
    /// remains stable across restarts. The `chain_id` scopes gossipsub
    /// topics and sync protocol to prevent cross-chain message leakage.
    pub fn new(
        data_dir: &Path,
        p2p_port: u16,
        bootstrap_addrs: Vec<Multiaddr>,
        chain_id: &str,
    ) -> Result<(Self, mpsc::UnboundedReceiver<NetworkEvent>, mpsc::UnboundedSender<P2pCommand>), Box<dyn std::error::Error>> {
        let local_key = load_or_generate_keypair(data_dir)?;
        let local_peer_id = local_key.public().to_peer_id();

        tracing::info!(%local_peer_id, %chain_id, "P2P identity created");

        let behaviour = ClawBehaviour::new(&local_key, chain_id)?;

        let mut swarm = SwarmBuilder::with_existing_identity(local_key)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_behaviour(|_| Ok(behaviour))
            .map_err(|e| format!("build swarm: {e}"))?
            .with_swarm_config(|c| {
                // Keep connections alive indefinitely — gossipsub heartbeat handles liveness.
                // Peers are removed only on actual connection failure or explicit disconnect.
                c.with_idle_connection_timeout(std::time::Duration::from_secs(u64::MAX / 2))
            })
            .build();

        // Subscribe to chain_id-scoped gossip topics
        let tx_topic = gossipsub::IdentTopic::new(protocol::topic_tx(chain_id));
        let block_topic = gossipsub::IdentTopic::new(protocol::topic_block(chain_id));
        let vote_topic = gossipsub::IdentTopic::new(protocol::topic_vote(chain_id));
        let checkin_topic = gossipsub::IdentTopic::new(protocol::topic_miner_checkin(chain_id));
        swarm.behaviour_mut().gossipsub.subscribe(&tx_topic)?;
        swarm.behaviour_mut().gossipsub.subscribe(&block_topic)?;
        swarm.behaviour_mut().gossipsub.subscribe(&vote_topic)?;
        swarm.behaviour_mut().gossipsub.subscribe(&checkin_topic)?;

        // Listen
        let listen_addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{p2p_port}").parse()?;
        swarm.listen_on(listen_addr)?;

        // Dial bootstrap peers (deduplicated)
        let mut seen_addrs = HashSet::new();
        let mut stored_bootstrap = Vec::new();
        for addr in bootstrap_addrs {
            if seen_addrs.insert(addr.clone()) {
                tracing::info!(%addr, "Dialing bootstrap peer");
                if let Err(e) = swarm.dial(addr.clone()) {
                    tracing::warn!(%addr, error=%e, "Failed to dial bootstrap");
                }
                stored_bootstrap.push(addr);
            }
        }

        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (command_tx, command_rx) = mpsc::unbounded_channel();

        Ok((
            Self {
                swarm,
                event_tx,
                command_rx,
                peers: HashSet::new(),
                incompatible_peers: HashSet::new(),
                tx_topic,
                block_topic,
                vote_topic,
                checkin_topic,
                bootstrap_addrs: stored_bootstrap,
            },
            event_rx,
            command_tx,
        ))
    }

    /// Broadcast a transaction to the network.
    pub fn broadcast_tx(&mut self, tx: &claw_types::transaction::Transaction) {
        let msg = GossipMessage::NewTx(tx.clone());
        let bytes = borsh::to_vec(&msg).expect("serialize gossip msg");
        if bytes.len() > protocol::MAX_P2P_MESSAGE_SIZE {
            tracing::warn!(
                size = bytes.len(),
                max = protocol::MAX_P2P_MESSAGE_SIZE,
                "Dropping outbound tx: exceeds max message size"
            );
            return;
        }
        if let Err(e) = self
            .swarm
            .behaviour_mut()
            .gossipsub
            .publish(self.tx_topic.clone(), bytes)
        {
            tracing::debug!(error=%e, "Failed to publish tx (no peers yet)");
        }
    }

    /// Broadcast a block to the network.
    pub fn broadcast_block(&mut self, block: &claw_types::block::Block) {
        let msg = GossipMessage::NewBlock(block.clone());
        let bytes = borsh::to_vec(&msg).expect("serialize gossip msg");
        if bytes.len() > protocol::MAX_P2P_MESSAGE_SIZE {
            tracing::warn!(
                size = bytes.len(),
                max = protocol::MAX_P2P_MESSAGE_SIZE,
                "Dropping outbound block: exceeds max message size"
            );
            return;
        }
        if let Err(e) = self
            .swarm
            .behaviour_mut()
            .gossipsub
            .publish(self.block_topic.clone(), bytes)
        {
            tracing::debug!(error=%e, "Failed to publish block (no peers yet)");
        }
    }

    /// Broadcast a BFT vote to the network.
    pub fn broadcast_vote(&mut self, vote: &BlockVote) {
        let msg = GossipMessage::Vote(vote.clone());
        let bytes = borsh::to_vec(&msg).expect("serialize gossip msg");
        if bytes.len() > protocol::MAX_P2P_MESSAGE_SIZE {
            tracing::warn!(
                size = bytes.len(),
                max = protocol::MAX_P2P_MESSAGE_SIZE,
                "Dropping outbound vote: exceeds max message size"
            );
            return;
        }
        if let Err(e) = self
            .swarm
            .behaviour_mut()
            .gossipsub
            .publish(self.vote_topic.clone(), bytes)
        {
            tracing::debug!(error=%e, "Failed to publish vote (no peers yet)");
        }
    }

    /// Broadcast a miner checkin witness (V3).
    /// Falls back to request-response direct push if gossipsub publish fails.
    pub fn broadcast_checkin(&mut self, witness: &claw_types::state::MinerCheckinWitness) {
        // Fix 4: diagnostic logging before publish
        let checkin_hash = self.checkin_topic.hash();
        let topic_peer_count = self.swarm.behaviour()
            .gossipsub.all_peers()
            .filter(|(_, topics)| topics.contains(&&checkin_hash))
            .count();
        let mesh_count = self.swarm.behaviour()
            .gossipsub.mesh_peers(&checkin_hash).count();
        let tcp_connected = self.swarm.connected_peers().count();
        tracing::debug!(
            topic_peers = topic_peer_count,
            mesh_peers = mesh_count,
            tcp_connected = tcp_connected,
            mdns_peers = self.peers.len(),
            "Attempting checkin publish"
        );

        let msg = GossipMessage::MinerCheckin(witness.clone());
        let bytes = borsh::to_vec(&msg).expect("serialize gossip msg");
        if bytes.len() > protocol::MAX_P2P_MESSAGE_SIZE {
            return;
        }
        match self
            .swarm
            .behaviour_mut()
            .gossipsub
            .publish(self.checkin_topic.clone(), bytes)
        {
            Ok(_) => {
                tracing::debug!("Checkin published via gossipsub");
            }
            Err(e) => {
                // Fix 1b: fallback to request-response direct push
                let tcp_peers: Vec<PeerId> = self.swarm.connected_peers().copied().collect();
                tracing::warn!(
                    error=%e, tcp_connected=tcp_peers.len(),
                    "Gossipsub publish failed, falling back to direct push"
                );
                for peer_id in &tcp_peers {
                    let req = SyncRequest::PushMinerCheckin(witness.clone());
                    let req_bytes = borsh::to_vec(&req).expect("serialize sync request");
                    self.swarm.behaviour_mut().request_response
                        .send_request(peer_id, req_bytes);
                    tracing::info!(%peer_id, "Checkin pushed via request-response fallback");
                }
            }
        }
    }

    /// Send a sync request to a specific peer.
    pub fn send_sync_request(&mut self, peer: &PeerId, request: SyncRequest) {
        let bytes = borsh::to_vec(&request).expect("serialize sync request");
        self.swarm
            .behaviour_mut()
            .request_response
            .send_request(peer, bytes);
    }

    /// Respond to a sync request.
    pub fn send_sync_response(
        &mut self,
        channel: request_response::ResponseChannel<Vec<u8>>,
        response: SyncResponse,
    ) {
        let bytes = borsh::to_vec(&response).expect("serialize sync response");
        if let Err(e) = self
            .swarm
            .behaviour_mut()
            .request_response
            .send_response(channel, bytes)
        {
            tracing::warn!("Failed to send sync response: {e:?}");
        }
    }

    /// Get connected peer count.
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Alias for peer_count.
    pub fn connected_peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Get list of connected peer IDs.
    pub fn connected_peers(&self) -> Vec<PeerId> {
        self.peers.iter().copied().collect()
    }

    /// Run the network event loop. This drives the swarm, emits events,
    /// and processes commands from the chain engine.
    pub async fn run(&mut self) {
        let mut bootstrap_redial = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            tokio::select! {
                _ = bootstrap_redial.tick() => {
                    // Redial bootstrap peers that are not currently connected
                    for addr in &self.bootstrap_addrs {
                        if should_redial_bootstrap(addr, &self.peers) {
                            tracing::debug!(%addr, "Redialing disconnected bootstrap peer");
                            let _ = self.swarm.dial(addr.clone());
                        }
                    }

                    // Fix 3: If TCP-connected but no peers subscribed to checkin topic,
                    // re-subscribe to trigger fresh subscription exchange with connected peers.
                    let tcp_count = self.swarm.connected_peers().count();
                    if tcp_count > 0 {
                        let checkin_hash = self.checkin_topic.hash();
                        let has_topic_peers = self.swarm.behaviour()
                            .gossipsub.all_peers()
                            .any(|(_, topics)| topics.contains(&&checkin_hash));

                        if !has_topic_peers {
                            tracing::warn!(
                                tcp_connected = tcp_count,
                                "No peers subscribed to checkin topic, re-subscribing to trigger exchange"
                            );
                            let _ = self.swarm.behaviour_mut().gossipsub.unsubscribe(&self.checkin_topic);
                            let _ = self.swarm.behaviour_mut().gossipsub.subscribe(&self.checkin_topic);
                        }
                    }
                }
                Some(cmd) = self.command_rx.recv() => {
                    match cmd {
                        P2pCommand::SendSyncRequest { peer, request } => {
                            self.send_sync_request(&peer, request);
                        }
                        P2pCommand::SendSyncResponse { channel, response } => {
                            self.send_sync_response(channel, response);
                        }
                        P2pCommand::BroadcastBlock(block) => {
                            self.broadcast_block(&block);
                        }
                        P2pCommand::BroadcastVote(vote) => {
                            self.broadcast_vote(&vote);
                        }
                        P2pCommand::BroadcastTx(tx) => {
                            self.broadcast_tx(&tx);
                        }
                        P2pCommand::BroadcastCheckin(witness) => {
                            self.broadcast_checkin(&witness);
                        }
                    }
                }
                event = self.swarm.select_next_some() => {
            match event {
                SwarmEvent::Behaviour(ClawBehaviourEvent::Gossipsub(
                    gossipsub::Event::Message {
                        message, ..
                    },
                )) => {
                    // Enforce inbound message size limit
                    if message.data.len() > protocol::MAX_P2P_MESSAGE_SIZE {
                        tracing::warn!(
                            size = message.data.len(),
                            max = protocol::MAX_P2P_MESSAGE_SIZE,
                            "Dropping inbound gossip message: exceeds max size"
                        );
                        continue;
                    }
                    if let Ok(gossip_msg) =
                        GossipMessage::try_from_slice(&message.data)
                    {
                        match gossip_msg {
                            GossipMessage::NewTx(tx) => {
                                let _ = self.event_tx.send(NetworkEvent::NewTx(tx));
                            }
                            GossipMessage::NewBlock(block) => {
                                let _ = self.event_tx.send(NetworkEvent::NewBlock(block));
                            }
                            GossipMessage::Vote(vote) => {
                                let _ = self.event_tx.send(NetworkEvent::Vote(vote));
                            }
                            GossipMessage::MinerCheckin(witness) => {
                                let _ = self.event_tx.send(NetworkEvent::MinerCheckin(witness));
                            }
                        }
                    }
                }
                SwarmEvent::Behaviour(ClawBehaviourEvent::RequestResponse(
                    request_response::Event::Message { peer, message, .. },
                )) => match message {
                    request_response::Message::Request {
                        request, request_id, channel, ..
                    } => {
                        if let Ok(req) = SyncRequest::try_from_slice(&request) {
                            let _ = self.event_tx.send(NetworkEvent::SyncRequest {
                                peer,
                                request_id,
                                request: req,
                                channel,
                            });
                        }
                    }
                    request_response::Message::Response { response, .. } => {
                        match SyncResponse::try_from_slice(&response) {
                            Ok(resp) => {
                                tracing::debug!(%peer, response_size = response.len(), "Received sync response");
                                let _ = self.event_tx.send(NetworkEvent::SyncResponse {
                                    peer,
                                    response: resp,
                                });
                            }
                            Err(e) => {
                                tracing::warn!(%peer, response_size = response.len(), error = %e, "Failed to deserialize sync response");
                            }
                        }
                    }
                },
                SwarmEvent::Behaviour(ClawBehaviourEvent::RequestResponse(
                    request_response::Event::OutboundFailure { peer, error, .. },
                )) => {
                    tracing::warn!(%peer, ?error, "Sync request outbound failure");
                    // Peer doesn't support our chain_id-scoped sync protocol —
                    // likely a node on a different network (e.g. mainnet vs testnet).
                    // Blacklist to prevent mDNS from re-adding it repeatedly.
                    if matches!(error, request_response::OutboundFailure::UnsupportedProtocols) {
                        tracing::info!(%peer, "Evicting incompatible peer (UnsupportedProtocols)");
                        self.incompatible_peers.insert(peer);
                        self.peers.remove(&peer);
                        let _ = self.event_tx.send(NetworkEvent::PeerDisconnected(peer));
                    }
                }
                SwarmEvent::Behaviour(ClawBehaviourEvent::RequestResponse(
                    request_response::Event::InboundFailure { peer, error, .. },
                )) => {
                    tracing::warn!(%peer, ?error, "Sync request inbound failure");
                }
                SwarmEvent::Behaviour(ClawBehaviourEvent::RequestResponse(
                    request_response::Event::ResponseSent { peer, .. },
                )) => {
                    tracing::debug!(%peer, "Sync response sent successfully");
                }
                SwarmEvent::Behaviour(ClawBehaviourEvent::Mdns(mdns::Event::Discovered(
                    peers,
                ))) => {
                    for (peer_id, addr) in peers {
                        // Skip already-known peers
                        if self.peers.contains(&peer_id) {
                            continue;
                        }
                        // Skip peers previously identified as incompatible (different chain_id)
                        if self.incompatible_peers.contains(&peer_id) {
                            continue;
                        }
                        if self.peers.len() >= protocol::MAX_PEER_CONNECTIONS {
                            tracing::warn!(
                                max = protocol::MAX_PEER_CONNECTIONS,
                                "Max peer connections reached, ignoring mDNS discovery"
                            );
                            break;
                        }
                        tracing::info!(%peer_id, %addr, "mDNS: discovered peer");
                        // Dial the peer; gossipsub mesh membership is handled
                        // automatically via subscription exchange after connection.
                        self.peers.insert(peer_id);
                        let _ = self.event_tx.send(NetworkEvent::PeerConnected(peer_id));
                    }
                }
                SwarmEvent::Behaviour(ClawBehaviourEvent::Mdns(mdns::Event::Expired(
                    peers,
                ))) => {
                    for (peer_id, _) in peers {
                        self.peers.remove(&peer_id);
                        let _ = self.event_tx.send(NetworkEvent::PeerDisconnected(peer_id));
                    }
                }
                SwarmEvent::NewListenAddr { address, .. } => {
                    tracing::info!(%address, "Listening on");
                }
                SwarmEvent::ConnectionEstablished { peer_id, num_established, .. } => {
                    // Only count each peer once (ignore additional connections to same peer)
                    if num_established.get() == 1 {
                        if self.peers.len() >= protocol::MAX_PEER_CONNECTIONS {
                            tracing::warn!(
                                %peer_id,
                                max = protocol::MAX_PEER_CONNECTIONS,
                                "Max peer connections reached, not tracking new peer"
                            );
                        } else {
                            tracing::info!(%peer_id, "Peer connected");
                            // Do NOT add_explicit_peer — explicit peers are excluded from
                            // the gossipsub mesh and only receive flood-published messages.
                            // Let gossipsub naturally manage mesh membership via subscription
                            // exchange and GRAFT/PRUNE during heartbeats.
                            self.peers.insert(peer_id);
                            let _ = self.event_tx.send(NetworkEvent::PeerConnected(peer_id));
                        }
                    }
                }
                SwarmEvent::ConnectionClosed { peer_id, num_established, .. } => {
                    // Only remove when last connection to this peer closes
                    if num_established == 0 {
                        tracing::info!(%peer_id, "Peer disconnected");
                        self.peers.remove(&peer_id);
                        let _ = self.event_tx.send(NetworkEvent::PeerDisconnected(peer_id));
                    }
                }
                _ => {}
            }
                } // end match event
            } // end tokio::select!
        }
    }
}

/// Determine whether a bootstrap peer address should be redialed.
///
/// If the address contains an embedded PeerID (`/p2p/<id>`), only redial when
/// that specific peer is not in the connected set. If the address has no PeerID
/// (bare IP+port), always attempt redial — the caller cannot tell whether the
/// target is already connected, so it must try unconditionally.
pub fn should_redial_bootstrap(addr: &Multiaddr, connected_peers: &HashSet<PeerId>) -> bool {
    let peer_id = addr.iter().find_map(|p| {
        if let libp2p::multiaddr::Protocol::P2p(id) = p { Some(id) } else { None }
    });
    match peer_id {
        Some(pid) => !connected_peers.contains(&pid),
        None => true,
    }
}

use futures::StreamExt;

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn make_addr(s: &str) -> Multiaddr {
        s.parse().unwrap()
    }

    fn make_peer_id(s: &str) -> PeerId {
        PeerId::from_str(s).unwrap()
    }

    const HETZNER_PEER: &str = "12D3KooWGVXR1MTGqQfnxgpguaiKGEtxc8sFYMbkuJkHdfnuHobG";

    #[test]
    fn bare_address_always_redials_even_with_other_peers() {
        let addr = make_addr("/ip4/39.102.144.231/tcp/9711");
        let mut peers = HashSet::new();
        // Even with unrelated peers connected, bare address should redial
        peers.insert(make_peer_id("12D3KooWNwBWp2mdsBMXpB7fnteNBkQTAQkXsZGwGzF2zjHpTGyT"));
        assert!(should_redial_bootstrap(&addr, &peers));
    }

    #[test]
    fn bare_address_redials_when_no_peers() {
        let addr = make_addr("/ip4/178.156.162.162/tcp/9711");
        let peers = HashSet::new();
        assert!(should_redial_bootstrap(&addr, &peers));
    }

    #[test]
    fn address_with_peer_id_skips_when_connected() {
        let addr = make_addr(&format!("/ip4/178.156.162.162/tcp/9711/p2p/{HETZNER_PEER}"));
        let mut peers = HashSet::new();
        peers.insert(make_peer_id(HETZNER_PEER));
        assert!(!should_redial_bootstrap(&addr, &peers));
    }

    #[test]
    fn address_with_peer_id_redials_when_disconnected() {
        let addr = make_addr(&format!("/ip4/178.156.162.162/tcp/9711/p2p/{HETZNER_PEER}"));
        let mut peers = HashSet::new();
        // Different peer connected, target peer is not
        peers.insert(make_peer_id("12D3KooWNwBWp2mdsBMXpB7fnteNBkQTAQkXsZGwGzF2zjHpTGyT"));
        assert!(should_redial_bootstrap(&addr, &peers));
    }

    #[test]
    fn address_with_peer_id_redials_when_empty() {
        let addr = make_addr(&format!("/ip4/178.156.162.162/tcp/9711/p2p/{HETZNER_PEER}"));
        let peers = HashSet::new();
        assert!(should_redial_bootstrap(&addr, &peers));
    }
}
