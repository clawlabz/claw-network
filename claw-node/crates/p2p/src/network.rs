//! P2P network manager — wraps libp2p swarm and exposes high-level events.

use borsh::BorshDeserialize;
use libp2p::{
    gossipsub, identity, mdns, noise,
    request_response::{self},
    swarm::SwarmEvent,
    tcp, yamux, Multiaddr, PeerId, Swarm, SwarmBuilder,
};
use std::collections::HashSet;
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
    /// A sync request from a peer.
    SyncRequest {
        peer: PeerId,
        request_id: request_response::InboundRequestId,
        request: SyncRequest,
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

/// P2P Network handle for interacting with the swarm.
pub struct P2pNetwork {
    swarm: Swarm<ClawBehaviour>,
    event_tx: mpsc::UnboundedSender<NetworkEvent>,
    peers: HashSet<PeerId>,
    tx_topic: gossipsub::IdentTopic,
    block_topic: gossipsub::IdentTopic,
}

impl P2pNetwork {
    /// Create a new P2P network.
    pub fn new(
        p2p_port: u16,
        bootstrap_addrs: Vec<Multiaddr>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<NetworkEvent>), Box<dyn std::error::Error>> {
        let local_key = identity::Keypair::generate_ed25519();
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
                c.with_idle_connection_timeout(std::time::Duration::from_secs(60))
            })
            .build();

        // Subscribe to gossip topics
        let tx_topic = gossipsub::IdentTopic::new(TOPIC_TX);
        let block_topic = gossipsub::IdentTopic::new(TOPIC_BLOCK);
        swarm.behaviour_mut().gossipsub.subscribe(&tx_topic)?;
        swarm.behaviour_mut().gossipsub.subscribe(&block_topic)?;

        // Listen
        let listen_addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{p2p_port}").parse()?;
        swarm.listen_on(listen_addr)?;

        // Dial bootstrap peers
        for addr in bootstrap_addrs {
            tracing::info!(%addr, "Dialing bootstrap peer");
            if let Err(e) = swarm.dial(addr.clone()) {
                tracing::warn!(%addr, error=%e, "Failed to dial bootstrap");
            }
        }

        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Ok((
            Self {
                swarm,
                event_tx,
                peers: HashSet::new(),
                tx_topic,
                block_topic,
            },
            event_rx,
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

    /// Run the network event loop. This drives the swarm and emits events.
    pub async fn run(&mut self) {
        loop {
            match self.swarm.select_next_some().await {
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
                            });
                        }
                        // Note: channel must be used to respond — handled by chain engine
                        // For now we drop it; integration in M3.3 will wire this up
                        drop(channel);
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
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    if self.peers.len() >= protocol::MAX_PEER_CONNECTIONS {
                        tracing::warn!(
                            %peer_id,
                            max = protocol::MAX_PEER_CONNECTIONS,
                            "Max peer connections reached, not tracking new peer"
                        );
                    } else {
                        self.peers.insert(peer_id);
                    }
                }
                SwarmEvent::ConnectionClosed { peer_id, .. } => {
                    self.peers.remove(&peer_id);
                }
                _ => {}
            }
        }
    }
}

use futures::StreamExt;
