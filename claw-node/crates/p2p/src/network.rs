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
    /// Broadcast a BFT vote to the network.
    BroadcastVote(BlockVote),
}

/// P2P Network handle for interacting with the swarm.
pub struct P2pNetwork {
    swarm: Swarm<ClawBehaviour>,
    event_tx: mpsc::UnboundedSender<NetworkEvent>,
    command_rx: mpsc::UnboundedReceiver<P2pCommand>,
    peers: HashSet<PeerId>,
    tx_topic: gossipsub::IdentTopic,
    block_topic: gossipsub::IdentTopic,
    vote_topic: gossipsub::IdentTopic,
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
    /// Create a new P2P network.
    /// Returns (network, event_receiver, command_sender).
    ///
    /// The `data_dir` is used to persist the P2P keypair so the peer ID
    /// remains stable across restarts.
    pub fn new(
        data_dir: &Path,
        p2p_port: u16,
        bootstrap_addrs: Vec<Multiaddr>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<NetworkEvent>, mpsc::UnboundedSender<P2pCommand>), Box<dyn std::error::Error>> {
        let local_key = load_or_generate_keypair(data_dir)?;
        let local_peer_id = local_key.public().to_peer_id();

        tracing::info!(%local_peer_id, "P2P identity created");

        let behaviour = ClawBehaviour::new(&local_key)?;

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

        // Subscribe to gossip topics
        let tx_topic = gossipsub::IdentTopic::new(TOPIC_TX);
        let block_topic = gossipsub::IdentTopic::new(TOPIC_BLOCK);
        let vote_topic = gossipsub::IdentTopic::new(TOPIC_VOTE);
        swarm.behaviour_mut().gossipsub.subscribe(&tx_topic)?;
        swarm.behaviour_mut().gossipsub.subscribe(&block_topic)?;
        swarm.behaviour_mut().gossipsub.subscribe(&vote_topic)?;

        // Listen
        let listen_addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{p2p_port}").parse()?;
        swarm.listen_on(listen_addr)?;

        // Dial bootstrap peers (deduplicated)
        let mut seen_addrs = HashSet::new();
        for addr in bootstrap_addrs {
            if seen_addrs.insert(addr.clone()) {
                tracing::info!(%addr, "Dialing bootstrap peer");
                if let Err(e) = swarm.dial(addr.clone()) {
                    tracing::warn!(%addr, error=%e, "Failed to dial bootstrap");
                }
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
                tx_topic,
                block_topic,
                vote_topic,
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
        loop {
            tokio::select! {
                Some(cmd) = self.command_rx.recv() => {
                    match cmd {
                        P2pCommand::SendSyncRequest { peer, request } => {
                            self.send_sync_request(&peer, request);
                        }
                        P2pCommand::SendSyncResponse { channel, response } => {
                            self.send_sync_response(channel, response);
                        }
                        P2pCommand::BroadcastVote(vote) => {
                            self.broadcast_vote(&vote);
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
                        if let Ok(resp) = SyncResponse::try_from_slice(&response) {
                            let _ = self.event_tx.send(NetworkEvent::SyncResponse {
                                peer,
                                response: resp,
                            });
                        }
                    }
                },
                SwarmEvent::Behaviour(ClawBehaviourEvent::Mdns(mdns::Event::Discovered(
                    peers,
                ))) => {
                    for (peer_id, addr) in peers {
                        // Skip already-known peers
                        if self.peers.contains(&peer_id) {
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
                        self.swarm
                            .behaviour_mut()
                            .gossipsub
                            .add_explicit_peer(&peer_id);
                        self.peers.insert(peer_id);
                        let _ = self.event_tx.send(NetworkEvent::PeerConnected(peer_id));
                    }
                }
                SwarmEvent::Behaviour(ClawBehaviourEvent::Mdns(mdns::Event::Expired(
                    peers,
                ))) => {
                    for (peer_id, _) in peers {
                        self.swarm
                            .behaviour_mut()
                            .gossipsub
                            .remove_explicit_peer(&peer_id);
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
                            self.swarm
                                .behaviour_mut()
                                .gossipsub
                                .add_explicit_peer(&peer_id);
                            self.peers.insert(peer_id);
                            let _ = self.event_tx.send(NetworkEvent::PeerConnected(peer_id));
                        }
                    }
                }
                SwarmEvent::ConnectionClosed { peer_id, num_established, .. } => {
                    // Only remove when last connection to this peer closes
                    if num_established == 0 {
                        tracing::info!(%peer_id, "Peer disconnected");
                        self.swarm
                            .behaviour_mut()
                            .gossipsub
                            .remove_explicit_peer(&peer_id);
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

use futures::StreamExt;
