use crate::gateway::auth::PairingStore;
use serde::Deserialize;
use serde_json::json;
use std::io::{self, Read, Write};
use std::sync::Arc;

#[derive(Deserialize)]
struct NativeMessage {
    action: Option<String>,
}

pub async fn start_native_messaging_host(auth_store: Arc<PairingStore>) {
    loop {
        // Read the 32-bit length prefix
        let mut len_bytes = [0u8; 4];
        if io::stdin().read_exact(&mut len_bytes).is_err() {
            break; // Stdin closed or error
        }

        // Convert length from native byte order (typically little-endian for Chrome)
        let len = u32::from_ne_bytes(len_bytes) as usize;

        let mut msg_buf = vec![0u8; len];
        if io::stdin().read_exact(&mut msg_buf).is_err() {
            break;
        }

        if let Ok(msg_str) = String::from_utf8(msg_buf)
            && let Ok(msg) = serde_json::from_str::<NativeMessage>(&msg_str)
            && msg.action.as_deref() == Some("get_token")
        {
            let token = auth_store.issue_token().await.unwrap_or_default();
            let response = json!({
                "token": token
            });

            let response_str = response.to_string();
            let response_len = response_str.len() as u32;
            let response_len_bytes = response_len.to_ne_bytes();

            let mut stdout = io::stdout();
            if stdout.write_all(&response_len_bytes).is_ok()
                && stdout.write_all(response_str.as_bytes()).is_ok()
            {
                let _ = stdout.flush();
            }
        }
    }
}
