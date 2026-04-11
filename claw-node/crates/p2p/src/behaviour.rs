//! Combined libp2p NetworkBehaviour.

use libp2p::{
    gossipsub, mdns,
    request_response::{self, ProtocolSupport},
    swarm::NetworkBehaviour,
    StreamProtocol,
};
use std::time::Duration;

use crate::protocol::MAX_P2P_MESSAGE_SIZE;

/// Combined network behaviour for ClawNetwork.
#[derive(NetworkBehaviour)]
pub struct ClawBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub request_response: request_response::Behaviour<SyncCodec>,
    pub mdns: mdns::tokio::Behaviour,
}

/// Simple codec that sends/receives raw bytes (borsh-serialized externally).
#[derive(Debug, Clone, Default)]
pub struct SyncCodec;

#[async_trait::async_trait]
impl request_response::Codec for SyncCodec {
    type Protocol = StreamProtocol;
    type Request = Vec<u8>;
    type Response = Vec<u8>;

    async fn read_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
    ) -> std::io::Result<Self::Request>
    where
        T: futures::AsyncRead + Unpin + Send,
    {
        // Read length-prefixed message (4-byte big-endian length + payload)
        let mut len_buf = [0u8; 4];
        futures::AsyncReadExt::read_exact(&mut *io, &mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > MAX_P2P_MESSAGE_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "message too large",
            ));
        }
        let mut buf = vec![0u8; len];
        futures::AsyncReadExt::read_exact(&mut *io, &mut buf).await?;
        Ok(buf)
    }

    async fn read_response<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
    ) -> std::io::Result<Self::Response>
    where
        T: futures::AsyncRead + Unpin + Send,
    {
        self.read_request(protocol, io).await
    }

    async fn write_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
        req: Self::Request,
    ) -> std::io::Result<()>
    where
        T: futures::AsyncWrite + Unpin + Send,
    {
        let len = (req.len() as u32).to_be_bytes();
        futures::AsyncWriteExt::write_all(&mut *io, &len).await?;
        futures::AsyncWriteExt::write_all(&mut *io, &req).await?;
        Ok(())
    }

    async fn write_response<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
        resp: Self::Response,
    ) -> std::io::Result<()>
    where
        T: futures::AsyncWrite + Unpin + Send,
    {
        self.write_request(protocol, io, resp).await
    }
}

impl ClawBehaviour {
    /// Create a new ClawBehaviour with chain_id-scoped protocols.
    ///
    /// The `chain_id` is used to scope gossipsub topics and request_response
    /// protocols so that different chains (mainnet/testnet) on the same
    /// network do not exchange messages.
    pub fn new(local_key: &libp2p::identity::Keypair, chain_id: &str) -> Result<Self, Box<dyn std::error::Error>> {
        // Gossipsub config
        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(1))
            .validation_mode(gossipsub::ValidationMode::Strict)
            .max_transmit_size(MAX_P2P_MESSAGE_SIZE)
            // Tuned for small networks (<10 nodes).
            // Defaults (mesh_n=6, mesh_n_low=4) cause excessive GRAFT churn
            // when fewer than 4 peers are available per topic.
            .mesh_n(3)
            .mesh_n_low(1)
            .mesh_n_high(6)
            .mesh_outbound_min(1)
            .build()
            .map_err(|e| format!("gossipsub config: {e}"))?;

        let gossipsub = gossipsub::Behaviour::new(
            gossipsub::MessageAuthenticity::Signed(local_key.clone()),
            gossipsub_config,
        )
        .map_err(|e| format!("gossipsub: {e}"))?;

        // Request-response for sync — chain_id scoped protocol
        let sync_proto_str = crate::protocol::sync_protocol(chain_id);
        let sync_proto = StreamProtocol::try_from_owned(sync_proto_str)
            .map_err(|e| format!("invalid sync protocol: {e}"))?;
        let request_response = request_response::Behaviour::new(
            [(sync_proto, ProtocolSupport::Full)],
            request_response::Config::default()
                .with_request_timeout(Duration::from_secs(30)),
        );

        // mDNS for local discovery
        let mdns = mdns::tokio::Behaviour::new(
            mdns::Config::default(),
            local_key.public().to_peer_id(),
        )?;

        Ok(Self {
            gossipsub,
            request_response,
            mdns,
        })
    }
}
