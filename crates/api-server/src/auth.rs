use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::Response;
use subtle::ConstantTimeEq;

use crate::AppState;
use crate::error::ApiError;

pub async fn require_token(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let presented = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    // Constant-time compare so response timing does not leak how much of the token matched.
    let allowed = match presented {
        Some(token) => token.as_bytes().ct_eq(state.token.as_bytes()).into(),
        None => false,
    };
    if !allowed {
        return Err(ApiError::unauthorized());
    }
    return Ok(next.run(request).await);
}
