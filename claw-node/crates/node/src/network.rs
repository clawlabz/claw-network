//! Network presets: built-in bootstrap nodes and chain IDs for each network.

use clap::ValueEnum;

#[derive(Clone, Debug, ValueEnum)]
pub enum Network {
    /// Public mainnet
    Mainnet,
    /// Public testnet (default for new users)
    Testnet,
    /// Local development network (single-node, no P2P)
    Devnet,
}

pub struct NetworkConfig {
    pub chain_id: &'static str,
    pub bootstrap_peers: Vec<&'static str>,
    pub is_local: bool,
    pub faucet_enabled: bool,
}

impl Network {
    pub fn config(&self) -> NetworkConfig {
        match self {
            Network::Mainnet => NetworkConfig {
                chain_id: "claw-mainnet-1",
                bootstrap_peers: vec![
                    // Will be populated when mainnet launches
                    // "/ip4/<US_EAST_IP>/tcp/9711",
                    // "/ip4/<EU_WEST_IP>/tcp/9711",
                    // "/ip4/<AP_SOUTHEAST_IP>/tcp/9711",
                ],
                is_local: false,
                faucet_enabled: false,
            },
            Network::Testnet => NetworkConfig {
                chain_id: "claw-testnet-1",
                bootstrap_peers: vec![
                    "/ip4/39.102.144.231/tcp/9711",  // boot-1 Beijing (Alibaba Cloud)
                ],
                is_local: false,
                faucet_enabled: true,
            },
            Network::Devnet => NetworkConfig {
                chain_id: "claw-devnet",
                bootstrap_peers: vec![],
                is_local: true,
                faucet_enabled: true,
            },
        }
    }
}
