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

pub use network::{NetworkEvent, P2pNetwork};
pub use protocol::{SyncRequest, SyncResponse, MAX_P2P_MESSAGE_SIZE, MAX_PEER_CONNECTIONS};
