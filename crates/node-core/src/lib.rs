//! Thin wrapper around ldk-node for the Lightning Tool.
//!
//! Covers the node lifecycle, the on-chain wallet, peers, channels, BOLT11 invoices, payments and
//! live events. The seed stays inside ldk-node's storage directory and is never exposed here.

pub mod config;
pub mod error;
mod first_seen;
mod liquidity;
mod mempool;
mod payment_changes;
mod pending_onchain;
mod map;
pub mod node;
pub mod views;

pub use config::NodeConfig;
pub use error::NodeError;
pub use ldk_node::bitcoin::Network;
pub use node::LightningNode;
pub use views::*;
