//! Browser bridge: transports the running daemon's token, never issues one.
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::time::Duration;

const MAX_MESSAGE_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
struct NativeMessage {
    action: Option<String>,
}

#[derive(Deserialize)]
struct PairingReply {
    pairing_issued: bool,
    token: Option<String>,
}

async fn fetch_daemon_token() -> anyhow::Result<String> {
    let address: SocketAddr = std::env::var("GD3_HTTP_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:14371".to_owned())
        .parse()?;
    anyhow::ensure!(
        address.ip().is_loopback(),
        "Pairing requires a loopback endpoint"
    );
    let reply: PairingReply = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()?
        .post(format!("http://{address}/api/pairing/request"))
        .json(&json!({}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    anyhow::ensure!(reply.pairing_issued, "Daemon did not issue pairing");
    reply
        .token
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("Daemon returned an empty token"))
}

async fn respond(message: NativeMessage) -> Value {
    match message.action.as_deref() {
        Some("get_token") => match fetch_daemon_token().await {
            Ok(token) => json!({"token": token}),
            // Do not leak endpoint details or generate a fallback token.
            Err(_) => json!({"error": "daemon_unavailable", "retryable": true}),
        },
        _ => json!({"error": "unsupported_action", "retryable": false}),
    }
}

pub async fn start_native_messaging_host() {
    let mut len_bytes = [0u8; 4];
    while io::stdin().read_exact(&mut len_bytes).is_ok() {
        let len = u32::from_ne_bytes(len_bytes) as usize;
        if len == 0 || len > MAX_MESSAGE_BYTES {
            break;
        }
        let mut buffer = vec![0u8; len];
        if io::stdin().read_exact(&mut buffer).is_err() {
            break;
        }
        let response = match serde_json::from_slice::<NativeMessage>(&buffer) {
            Ok(message) => respond(message).await,
            Err(_) => json!({"error": "invalid_message", "retryable": false}),
        };
        let bytes = response.to_string().into_bytes();
        let mut stdout = io::stdout();
        if stdout
            .write_all(&(bytes.len() as u32).to_ne_bytes())
            .is_err()
            || stdout.write_all(&bytes).is_err()
            || stdout.flush().is_err()
        {
            break;
        }
    }
}
