//! Password storage: OS keychain in production, in-memory for tests.

use crate::{Error, Result};
use std::collections::HashMap;
use std::sync::Mutex;
use zeroize::Zeroizing;

const SERVICE: &str = "oxfer";

/// A secret string, zeroized on drop and never shown by `Debug`.
#[derive(Clone)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Self(Zeroizing::new(s.into()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

pub trait SecretStore: Send + Sync {
    fn get(&self, profile: &str) -> Result<Option<Secret>>;
    fn set(&self, profile: &str, secret: &Secret) -> Result<()>;
    /// Deleting a missing entry is not an error.
    fn delete(&self, profile: &str) -> Result<()>;
}

/// Stores secrets in the OS keychain under the service name `oxfer`.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringStore;

fn entry(profile: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, profile).map_err(|e| Error::Secret(e.to_string()))
}

impl SecretStore for KeyringStore {
    fn get(&self, profile: &str) -> Result<Option<Secret>> {
        match entry(profile)?.get_password() {
            Ok(pw) => Ok(Some(Secret::new(pw))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::Secret(e.to_string())),
        }
    }

    fn set(&self, profile: &str, secret: &Secret) -> Result<()> {
        entry(profile)?
            .set_password(secret.expose())
            .map_err(|e| Error::Secret(e.to_string()))
    }

    fn delete(&self, profile: &str) -> Result<()> {
        match entry(profile)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::Secret(e.to_string())),
        }
    }
}

#[derive(Debug, Default)]
pub struct MemoryStore(Mutex<HashMap<String, Secret>>);

impl SecretStore for MemoryStore {
    fn get(&self, profile: &str) -> Result<Option<Secret>> {
        Ok(self.0.lock().unwrap().get(profile).cloned())
    }

    fn set(&self, profile: &str, secret: &Secret) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(profile.to_string(), secret.clone());
        Ok(())
    }

    fn delete(&self, profile: &str) -> Result<()> {
        self.0.lock().unwrap().remove(profile);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_secret() {
        let s = Secret::new("hunter2");
        assert!(!format!("{s:?}").contains("hunter2"));
    }

    #[test]
    fn memory_store_roundtrip() {
        let st = MemoryStore::default();
        assert!(st.get("a").unwrap().is_none());
        st.set("a", &Secret::new("x")).unwrap();
        assert_eq!(st.get("a").unwrap().unwrap().expose(), "x");
        st.delete("a").unwrap();
        st.delete("a").unwrap();
        assert!(st.get("a").unwrap().is_none());
    }
}
