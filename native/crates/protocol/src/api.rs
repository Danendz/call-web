//! Request and response bodies of the `/api` endpoints, mirroring `shared/api.ts`. The native client signs in with
//! `client: "native"` and gets its session token in the response, sent back as `Authorization: Bearer <token>`.

use serde::Deserialize;

/// `USERNAME_RE` in api.ts; [`is_username`] is what checks it.
pub const USERNAME_PATTERN: &str = "^[a-z0-9_]{3,20}$";
pub const MIN_PASSWORD_LENGTH: usize = 10;
pub const MAX_PASSWORD_LENGTH: usize = 200;

pub fn is_username(name: &str) -> bool {
    (3..=20).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionResponse {
    pub username: String,
    /// Always there for the native client. Never log it.
    pub token: Option<String>,
    /// After register and recover only: shown once
    pub recovery_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MeResponse {
    pub username: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InvitesResponse {
    pub unused: u32,
    pub max: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewInviteResponse {
    pub code: String,
    pub expires_at: u64,
}

/// Why the server refused, from `{ "error": … }`. Codes this build doesn't know become `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApiErrorCode {
    BadRequest,
    NotFound,
    Unauthorized,
    Forbidden,
    RateLimited,
    Unavailable,
    InvalidCredentials,
    InvalidInvite,
    InvalidRecovery,
    UsernameTaken,
    InvalidUsername,
    WeakPassword,
    TooManyInvites,
    TooMany,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiError {
    pub error: ApiErrorCode,
}
