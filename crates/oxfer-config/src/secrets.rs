//! Password storage: OS keychain in production, in-memory for tests.

use crate::{Error, Result};
use std::collections::{BTreeMap, HashMap};
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

/// Stores secrets in the OS keychain.
///
/// All secrets live in a single keychain item (service `oxfer.vault`), so
/// macOS asks for access once instead of once per profile. Older versions
/// kept one item per profile under the service `oxfer`; those are moved into
/// the vault the first time they are read.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringStore;

const VAULT_SERVICE: &str = "oxfer.vault";
const VAULT_ACCOUNT: &str = "oxfer";

type Vault = HashMap<String, Secret>;

/// The vault as this process last read or wrote it.
static VAULT: Mutex<Option<Vault>> = Mutex::new(None);

fn secret_err(e: keyring::Error) -> Error {
    Error::Secret(e.to_string())
}

fn vault_entry() -> Result<keyring::Entry> {
    keyring::Entry::new(VAULT_SERVICE, VAULT_ACCOUNT).map_err(secret_err)
}

/// The pre-vault item holding one profile's secret.
fn legacy_entry(profile: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, profile).map_err(secret_err)
}

fn encode_vault(vault: &Vault) -> Result<Zeroizing<String>> {
    let map: BTreeMap<&str, &str> = vault
        .iter()
        .map(|(k, v)| (k.as_str(), v.expose()))
        .collect();
    toml::to_string(&map)
        .map(Zeroizing::new)
        .map_err(|e| Error::Secret(e.to_string()))
}

fn decode_vault(raw: &str) -> Result<Vault> {
    // The parse error would quote the offending line, i.e. a password.
    let map: BTreeMap<String, String> =
        toml::from_str(raw).map_err(|_| Error::Secret("keychain vault is corrupt".into()))?;
    Ok(map.into_iter().map(|(k, v)| (k, Secret::new(v))).collect())
}

fn read_vault() -> Result<Vault> {
    match vault_entry()?.get_password() {
        Ok(raw) => decode_vault(&Zeroizing::new(raw)),
        Err(keyring::Error::NoEntry) => Ok(Vault::new()),
        Err(e) => Err(secret_err(e)),
    }
}

/// Applies `f` to a fresh copy of the vault (another oxfer process may have
/// changed it) and writes it back if `f` reports a change.
fn modify_vault(f: impl FnOnce(&mut Vault) -> bool) -> Result<()> {
    let mut cache = VAULT.lock().unwrap();
    let mut vault = read_vault()?;
    if f(&mut vault) {
        vault_entry()?
            .set_password(&encode_vault(&vault)?)
            .map_err(secret_err)?;
    }
    *cache = Some(vault);
    Ok(())
}

impl SecretStore for KeyringStore {
    fn get(&self, profile: &str) -> Result<Option<Secret>> {
        {
            let mut cache = VAULT.lock().unwrap();
            if cache.is_none() {
                *cache = Some(read_vault()?);
            }
            if let Some(s) = cache.as_ref().and_then(|v| v.get(profile)) {
                return Ok(Some(s.clone()));
            }
        }
        let legacy = legacy_entry(profile)?;
        let secret = match legacy.get_password() {
            Ok(pw) => Secret::new(pw),
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(e) => return Err(secret_err(e)),
        };
        modify_vault(|v| {
            v.insert(profile.to_string(), secret.clone());
            true
        })?;
        // Already safe in the vault; a leftover item is only clutter.
        let _ = legacy.delete_credential();
        Ok(Some(secret))
    }

    fn set(&self, profile: &str, secret: &Secret) -> Result<()> {
        modify_vault(|v| {
            v.insert(profile.to_string(), secret.clone());
            true
        })
    }

    fn delete(&self, profile: &str) -> Result<()> {
        modify_vault(|v| v.remove(profile).is_some())?;
        // Otherwise `get` would fall back to it and bring the secret back.
        match legacy_entry(profile)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(secret_err(e)),
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
    fn vault_encoding_roundtrip() {
        let mut v = Vault::new();
        v.insert("prod".into(), Secret::new("p\"w=\n'x"));
        v.insert("user@host:22".into(), Secret::new(""));
        let back = decode_vault(&encode_vault(&v).unwrap()).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back["prod"].expose(), "p\"w=\n'x");
        assert_eq!(back["user@host:22"].expose(), "");
        assert!(decode_vault("").unwrap().is_empty());
    }

    #[test]
    fn corrupt_vault_error_hides_contents() {
        let err = decode_vault("hunter2 = = =").unwrap_err();
        assert!(!err.to_string().contains("hunter2"));
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
