use std::path::PathBuf;

use ldk_node::bitcoin::Network;

/// Everything needed to build the node. Chain data comes from a bitcoind RPC (Polar on regtest).
#[derive(Debug, Clone)]
pub struct NodeConfig {
    pub network: Network,
    pub data_dir: PathBuf,
    pub listen_address: String,
    pub alias: String,
    pub rpc_host: String,
    pub rpc_port: u16,
    pub rpc_user: String,
    pub rpc_password: String,
}
