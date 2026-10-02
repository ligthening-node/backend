use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use node_core::{Network, NodeConfig};

/// Largest payment the API will make without the operator raising the limit: 1,000,000 sat.
const DEFAULT_MAX_PAY_MSAT: &str = "1000000000";

pub struct ServerConfig {
    pub bind: SocketAddr,
    pub token: String,
    pub max_pay_msat: u64,
    pub node: NodeConfig,
}

impl ServerConfig {
    /// Reads `LN_*` environment variables. The API binds to loopback unless `LN_API_BIND=container`.
    pub fn from_env() -> Result<Self, String> {
        return Self::from_lookup(|key| env::var(key).ok());
    }

    /// Same as `from_env` with the variable source injected, so tests never touch the process
    /// environment.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let token = required(&get, "LN_API_TOKEN")?;
        if token.len() < 16 {
            return Err("LN_API_TOKEN must be at least 16 characters".to_string());
        }
        let network = match optional(&get, "LN_NETWORK", "regtest").as_str() {
            "regtest" => Network::Regtest,
            "signet" => Network::Signet,
            other => {
                return Err(format!(
                    "LN_NETWORK must be regtest or signet, got {other:?}"
                ));
            }
        };
        let api_port: u16 = parse(&get, "LN_API_PORT", "3001")?;
        // Docker publishes the port from inside a container, which only works on 0.0.0.0. The
        // compose file maps it to 127.0.0.1 on the host, so the API still stays local.
        let host: [u8; 4] = match optional(&get, "LN_API_BIND", "loopback").as_str() {
            "loopback" => [127, 0, 0, 1],
            "container" => [0, 0, 0, 0],
            other => {
                return Err(format!(
                    "LN_API_BIND must be loopback or container, got {other:?}"
                ));
            }
        };
        let node = NodeConfig {
            network,
            data_dir: PathBuf::from(optional(&get, "LN_DATA_DIR", "./ldk-data")),
            listen_address: optional(&get, "LN_LISTEN_ADDRESS", "127.0.0.1:9735"),
            alias: optional(&get, "LN_NODE_ALIAS", "lightning-tool"),
            rpc_host: optional(&get, "LN_RPC_HOST", "127.0.0.1"),
            rpc_port: parse(&get, "LN_RPC_PORT", "18443")?,
            rpc_user: optional(&get, "LN_RPC_USER", "polaruser"),
            rpc_password: required(&get, "LN_RPC_PASSWORD")?,
        };
        return Ok(Self {
            bind: SocketAddr::from((host, api_port)),
            token,
            max_pay_msat: parse(&get, "LN_MAX_PAY_MSAT", DEFAULT_MAX_PAY_MSAT)?,
            node,
        });
    }
}

fn required(get: &impl Fn(&str) -> Option<String>, key: &str) -> Result<String, String> {
    return get(key).ok_or_else(|| format!("{key} is required"));
}

fn optional(get: &impl Fn(&str) -> Option<String>, key: &str, default: &str) -> String {
    return get(key).unwrap_or_else(|| default.to_string());
}

fn parse<T: std::str::FromStr>(
    get: &impl Fn(&str) -> Option<String>,
    key: &str,
    default: &str,
) -> Result<T, String> {
    return optional(get, key, default)
        .parse()
        .map_err(|_| format!("{key} is not valid"));
}

// === Tests

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef";

    fn load(pairs: &[(&str, &str)]) -> Result<ServerConfig, String> {
        return ServerConfig::from_lookup(|key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        });
    }

    fn minimal() -> Vec<(&'static str, &'static str)> {
        return vec![("LN_API_TOKEN", TOKEN), ("LN_RPC_PASSWORD", "secret")];
    }

    fn with(key: &'static str, value: &'static str) -> Vec<(&'static str, &'static str)> {
        let mut pairs: Vec<_> = minimal().into_iter().filter(|(k, _)| *k != key).collect();
        pairs.push((key, value));
        return pairs;
    }

    #[test]
    fn defaults_are_loopback_regtest_and_a_one_million_sat_limit() {
        let config = load(&minimal()).unwrap();
        assert_eq!(config.bind, "127.0.0.1:3001".parse().unwrap());
        assert_eq!(config.max_pay_msat, 1_000_000_000);
        assert_eq!(config.node.network, Network::Regtest);
        assert_eq!(config.node.listen_address, "127.0.0.1:9735");
        assert_eq!(config.node.rpc_port, 18443);
    }

    #[test]
    fn the_token_is_required() {
        let pairs = [("LN_RPC_PASSWORD", "secret")];
        assert_eq!(load(&pairs).err().unwrap(), "LN_API_TOKEN is required");
    }

    #[test]
    fn short_tokens_are_rejected() {
        let err = load(&with("LN_API_TOKEN", "short")).err().unwrap();
        assert!(err.contains("at least 16 characters"), "{err}");
    }

    #[test]
    fn a_token_of_exactly_sixteen_characters_is_accepted() {
        assert!(load(&with("LN_API_TOKEN", "0123456789abcdef")).is_ok());
        assert!(load(&with("LN_API_TOKEN", "0123456789abcde")).is_err());
    }

    #[test]
    fn the_rpc_password_is_required() {
        let pairs = [("LN_API_TOKEN", TOKEN)];
        assert_eq!(load(&pairs).err().unwrap(), "LN_RPC_PASSWORD is required");
    }

    #[test]
    fn mainnet_is_not_a_supported_network() {
        for network in ["bitcoin", "mainnet", "testnet", ""] {
            let err = load(&with("LN_NETWORK", network)).err().unwrap();
            assert!(err.contains("regtest or signet"), "{network}: {err}");
        }
        let signet = load(&with("LN_NETWORK", "signet")).unwrap();
        assert_eq!(signet.node.network, Network::Signet);
    }

    #[test]
    fn the_api_binds_to_all_interfaces_only_inside_a_container() {
        let container = load(&with("LN_API_BIND", "container")).unwrap();
        assert_eq!(container.bind, "0.0.0.0:3001".parse().unwrap());
        let err = load(&with("LN_API_BIND", "0.0.0.0")).err().unwrap();
        assert!(err.contains("loopback or container"), "{err}");
    }

    #[test]
    fn bad_numbers_name_the_variable() {
        for (key, value) in [
            ("LN_API_PORT", "70000"),
            ("LN_API_PORT", "abc"),
            ("LN_RPC_PORT", "-1"),
            ("LN_MAX_PAY_MSAT", "1.5"),
            ("LN_MAX_PAY_MSAT", "-5"),
        ] {
            let err = load(&with(key, value)).err().unwrap();
            assert_eq!(err, format!("{key} is not valid"), "{key}={value}");
        }
    }

    #[test]
    fn overrides_are_applied() {
        let mut pairs = minimal();
        pairs.extend([
            ("LN_API_PORT", "3002"),
            ("LN_LISTEN_ADDRESS", "127.0.0.1:9736"),
            ("LN_NODE_ALIAS", "peer"),
            ("LN_DATA_DIR", "/tmp/peer"),
            ("LN_MAX_PAY_MSAT", "5000"),
        ]);
        let config = load(&pairs).unwrap();
        assert_eq!(config.bind.port(), 3002);
        assert_eq!(config.node.alias, "peer");
        assert_eq!(config.node.data_dir, PathBuf::from("/tmp/peer"));
        assert_eq!(config.max_pay_msat, 5000);
    }
}
