use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use node_core::NodeError;
use serde::Serialize;
use serde_json::Value;

/// Every failure leaves the API as `{ "error": { "code", "message", "details"? } }`.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    details: Option<Value>,
}

#[derive(Serialize)]
struct Envelope<'a> {
    error: Body<'a>,
}

#[derive(Serialize)]
struct Body<'a> {
    code: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<&'a Value>,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: String) -> Self {
        return Self {
            status,
            code,
            message,
            details: None,
        };
    }

    pub fn unauthorized() -> Self {
        return Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing or invalid bearer token".to_string(),
        );
    }

    pub fn bad_request(message: String) -> Self {
        return Self::new(StatusCode::BAD_REQUEST, "invalid_input", message);
    }

    pub fn decode_failed(message: String) -> Self {
        return Self::new(StatusCode::UNPROCESSABLE_ENTITY, "decode_failed", message);
    }

    /// The pre-payment checks said no. `details` carries the full validation report.
    pub fn payment_refused(message: String, details: Value) -> Self {
        return Self {
            details: Some(details),
            ..Self::new(StatusCode::UNPROCESSABLE_ENTITY, "payment_refused", message)
        };
    }

    pub fn internal(message: String) -> Self {
        return Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", message);
    }
}

impl From<NodeError> for ApiError {
    fn from(err: NodeError) -> Self {
        return match err {
            NodeError::InvalidInput(message) => Self::bad_request(message),
            // The node is fine; the payment just does not fit its channels. A caller error, not a 500.
            NodeError::Liquidity(message) => Self::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "insufficient_liquidity",
                message,
            ),
            other => Self::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "node_error",
                other.to_string(),
            ),
        };
    }
}

impl From<JsonRejection> for ApiError {
    fn from(err: JsonRejection) -> Self {
        return Self::bad_request(err.body_text());
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Envelope {
            error: Body {
                code: self.code,
                message: &self.message,
                details: self.details.as_ref(),
            },
        };
        return (self.status, Json(body)).into_response();
    }
}

// === Tests

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn render(error: ApiError) -> (StatusCode, Value) {
        let response = error.into_response();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        return (status, serde_json::from_slice(&bytes).unwrap());
    }

    #[tokio::test]
    async fn every_error_uses_the_envelope_with_its_status() {
        let cases = [
            (ApiError::unauthorized(), StatusCode::UNAUTHORIZED, "unauthorized"),
            (
                ApiError::bad_request("x".to_string()),
                StatusCode::BAD_REQUEST,
                "invalid_input",
            ),
            (
                ApiError::decode_failed("x".to_string()),
                StatusCode::UNPROCESSABLE_ENTITY,
                "decode_failed",
            ),
            (
                ApiError::internal("x".to_string()),
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
            ),
        ];
        for (error, status, code) in cases {
            let (got_status, body) = render(error).await;
            assert_eq!(got_status, status);
            assert_eq!(body["error"]["code"], code);
            assert!(body["error"]["message"].is_string());
            assert!(body["error"].get("details").is_none());
        }
    }

    #[tokio::test]
    async fn a_refused_payment_carries_the_report() {
        let error = ApiError::payment_refused("no".to_string(), json!({ "verdict": "not_payable" }));
        let (status, body) = render(error).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"]["code"], "payment_refused");
        assert_eq!(body["error"]["details"]["verdict"], "not_payable");
    }

    #[tokio::test]
    async fn node_errors_split_into_client_and_server_faults() {
        let (status, body) = render(NodeError::InvalidInput("bad key".to_string()).into()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "invalid_input");
        assert_eq!(body["error"]["message"], "bad key");

        for error in [
            NodeError::Node("channel not found".to_string()),
            NodeError::Build("disk".to_string()),
            NodeError::Config("port".to_string()),
        ] {
            let (status, body) = render(error.into()).await;
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(body["error"]["code"], "node_error");
        }
    }
}
