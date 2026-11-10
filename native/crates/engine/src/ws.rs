//! Opening a WebSocket to our server, with the account's session when there is one.

use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use url::Url;

/// The upgrade request for `url`, carrying `session` as a bearer token: the server checks it for private rooms and
/// `/ws/user`, and counts a signed-in user's sockets against the account instead of the tighter anonymous limits.
pub fn request(url: &Url, session: Option<&str>) -> Result<Request, String> {
    let mut request = url.as_str().into_client_request().map_err(|e| e.to_string())?;
    if let Some(token) = session {
        let value =
            HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| "session token isn't a header value")?;
        request.headers_mut().insert(AUTHORIZATION, value);
    }
    Ok(request)
}

/// The server's address with a `ws:` or `wss:` scheme and this path.
pub fn ws_url(server: &Url, path: &str) -> Url {
    let mut url = server.clone();
    // Only http and https servers are accepted at startup, so this can't fail
    let _ = url.set_scheme(if server.scheme() == "https" { "wss" } else { "ws" });
    url.set_path(path);
    url.set_query(None);
    url
}
