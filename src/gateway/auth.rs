use anyhow::Result;
use base64::{Engine as _, engine::general_purpose};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct PairingStore {
    token_path: PathBuf,
    current_token: Arc<RwLock<Option<String>>>,
}

impl PairingStore {
    pub fn new(token_path: impl Into<PathBuf>) -> Self {
        let token_path = token_path.into();
        let current_token = std::fs::read_to_string(&token_path)
            .ok()
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty());

        Self {
            token_path,
            current_token: Arc::new(RwLock::new(current_token)),
        }
    }

    pub async fn issue_token(&self) -> Result<String> {
        if let Some(existing_token) = self.current_token.read().await.clone() {
            return Ok(existing_token);
        }

        let token = generate_pairing_token()?;
        if let Some(parent) = self.token_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&self.token_path, &token).await?;
        *self.current_token.write().await = Some(token.clone());
        Ok(token)
    }

    pub async fn validate(&self, token: &str) -> bool {
        let token = token.trim();
        if token.is_empty() {
            return false;
        }
        self.current_token
            .read()
            .await
            .as_deref()
            .map(|stored| stored == token)
            .unwrap_or(false)
    }

    pub async fn revoke(&self) -> Result<()> {
        *self.current_token.write().await = None;
        if Path::new(&self.token_path).exists() {
            tokio::fs::remove_file(&self.token_path).await?;
        }
        Ok(())
    }
}

fn generate_pairing_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)?;
    Ok(format!(
        "gd3_{}",
        general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_token_has_expected_prefix() {
        let token = generate_pairing_token().unwrap();
        assert!(token.starts_with("gd3_"));
        assert!(token.len() > 32);
    }

    #[tokio::test]
    async fn issuing_token_is_idempotent_until_revoked() {
        let token_path =
            std::env::temp_dir().join(format!("gd3_pairing_token_test_{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&token_path);
        let store = PairingStore::new(&token_path);

        let first = store.issue_token().await.unwrap();
        let second = store.issue_token().await.unwrap();

        assert_eq!(first, second);
        assert!(store.validate(&first).await);

        store.revoke().await.unwrap();
        let third = store.issue_token().await.unwrap();

        assert_ne!(first, third);
        assert!(!store.validate(&first).await);
        assert!(store.validate(&third).await);

        let _ = std::fs::remove_file(&token_path);
    }
}
