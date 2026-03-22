//! P2P networking layer for ClawNetwork using libp2p.
//!
//! Provides:
//! - Gossipsub for tx and block broadcasting
//! - Request-response for block sync
//! - mDNS for local peer discovery
//! - Bootstrap nodes for public network discovery

mod behaviour;
mod network;
mod protocol;

pub use network::{NetworkEvent, P2pCommand, P2pNetwork};
pub use protocol::{BlockVote, SyncRequest, SyncResponse, MAX_P2P_MESSAGE_SIZE, MAX_PEER_CONNECTIONS};

/// Re-export ResponseChannel so the chain engine can send sync responses.
pub use libp2p::request_response::ResponseChannel;

/// Re-export PeerId so the chain engine can track connected peers.
pub use libp2p::PeerId;
