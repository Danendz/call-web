//! The `/api` endpoints, called with ureq on a blocking thread: a handful of small JSON requests.

use std::time::Duration;

use protocol::{ApiError, ApiErrorCode};
use serde::de::DeserializeOwned;
use serde_json::Value;
use url::Url;

use super::SessionToken;

pub struct Api {
    agent: ureq::Agent,
    server: Url,
}

impl Api {
    pub fn new(server: Url) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        Self { agent, server }
    }

    /// POSTs `body` (or GETs without one) and returns the response's JSON; `Value::Null` for an empty answer.
    /// Every failure, the network's too, comes back as the server's error code.
    pub fn call(&self, path: &str, session: Option<&SessionToken>, body: Option<Value>) -> Result<Value, ApiErrorCode> {
        let url = self.server.join(path).map_err(|_| ApiErrorCode::BadRequest)?;
        let auth = session.map(|s| format!("Bearer {}", s.0));
        let result = match body {
            Some(body) => {
                let mut request = self.agent.post(url.as_str()).header("content-type", "application/json");
                if let Some(auth) = &auth {
                    request = request.header("authorization", auth);
                }
                request.send(body.to_string())
            }
            None => {
                let mut request = self.agent.get(url.as_str());
                if let Some(auth) = &auth {
                    request = request.header("authorization", auth);
                }
                request.call()
            }
        };
        let mut response = result.map_err(|e| {
            tracing::info!("api {path}: {e}");
            ApiErrorCode::Unavailable
        })?;
        let status = response.status();
        let text = response.body_mut().with_config().limit(64 * 1024).read_to_string().unwrap_or_default();
        if status.is_success() {
            return Ok(serde_json::from_str(&text).unwrap_or(Value::Null));
        }
        Err(serde_json::from_str::<ApiError>(&text).map(|e| e.error).unwrap_or(ApiErrorCode::Unavailable))
    }

    pub fn call_as<T: DeserializeOwned>(
        &self,
        path: &str,
        session: Option<&SessionToken>,
        body: Option<Value>,
    ) -> Result<T, ApiErrorCode> {
        serde_json::from_value(self.call(path, session, body)?).map_err(|_| ApiErrorCode::Unavailable)
    }
}
