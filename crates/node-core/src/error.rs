use thiserror::Error;

#[derive(Debug, Error)]
pub enum NodeError {
    #[error("invalid node configuration: {0}")]
    Config(String),

    #[error("could not build the node: {0}")]
    Build(String),

    /// The caller sent something unusable: a bad key, address, invoice or amount.
    #[error("{0}")]
    InvalidInput(String),

    /// The payment is valid but this node's channels cannot carry it. The message says why and what
    /// to do, so the caller can show it as is.
    #[error("{0}")]
    Liquidity(String),

    #[error("node operation failed: {0}")]
    Node(String),
}

impl From<ldk_node::NodeError> for NodeError {
    fn from(err: ldk_node::NodeError) -> Self {
        return match err {
            // ldk-node's own text here ("Failed to send the given payment.") names no cause.
            ldk_node::NodeError::PaymentSendingFailed => NodeError::Node(
                "the payment could not be sent. Check that the payee is reachable and that a usable channel can carry this amount."
                    .to_string(),
            ),
            other => NodeError::Node(other.to_string()),
        };
    }
}
