//! Named credentials that several profiles (or quick connects) can share.

use crate::profile::AuthMethod;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

/// Prefix of a credential's key in the [`SecretStore`](crate::SecretStore),
/// keeping it apart from the profiles' own passwords. Profile names may not
/// start with it.
pub const CREDENTIAL_SECRET_PREFIX: &str = "credential:";

/// A user name plus how to authenticate. A password lives in the secret
/// store under [`Credential::secret_key`], never in the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    pub auth: AuthMethod,
}

impl Credential {
    pub fn new(name: impl Into<String>, user: Option<String>, auth: AuthMethod) -> Self {
        Self {
            name: name.into(),
            user,
            auth,
        }
    }

    pub fn secret_key(&self) -> String {
        secret_key(&self.name)
    }

    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() || self.name.chars().any(char::is_control) {
            return Err(Error::InvalidCredential(format!(
                "bad credential name {:?}",
                self.name
            )));
        }
        if let AuthMethod::KeyFile { path } = &self.auth {
            if path.as_os_str().is_empty() {
                return Err(Error::InvalidCredential(format!(
                    "'{}': key file path is empty",
                    self.name
                )));
            }
        }
        Ok(())
    }
}

/// Secret store key of the credential `name`.
pub fn secret_key(name: &str) -> String {
    format!("{CREDENTIAL_SECRET_PREFIX}{name}")
}
