use serde::Serialize;

/// Error returned to the UI as `{ code, message }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceError {
    pub code: String,
    pub message: String,
}

pub type ServiceResult<T> = std::result::Result<T, ServiceError>;

impl ServiceError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid", message)
    }

    pub fn unknown_session(id: &str) -> Self {
        Self::new("unknownSession", format!("no open session {id:?}"))
    }

    pub fn unknown_transfer(id: u64) -> Self {
        Self::new("unknownTransfer", format!("no transfer #{id}"))
    }

    pub fn cancelled() -> Self {
        Self::new("cancelled", "cancelled by user")
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ServiceError {}

impl From<oxfer_core::Error> for ServiceError {
    fn from(e: oxfer_core::Error) -> Self {
        use oxfer_core::Error as E;
        let code = match &e {
            E::NotFound(_) => "notFound",
            E::PermissionDenied(_) => "permissionDenied",
            E::AlreadyExists(_) => "alreadyExists",
            E::Auth(_) => "auth",
            E::HostKey(_) => "hostKey",
            E::Connection(_) => "connection",
            E::Protocol(_) => "protocol",
            E::Unsupported(_) => "unsupported",
            E::Invalid(_) => "invalid",
            E::Cancelled => "cancelled",
            E::Internal(_) => "internal",
            E::Io(_) => "io",
        };
        Self::new(code, e.to_string())
    }
}

impl From<oxfer_config::Error> for ServiceError {
    fn from(e: oxfer_config::Error) -> Self {
        use oxfer_config::Error as E;
        let code = match &e {
            E::Invalid(_) | E::InvalidJump(_) => "invalid",
            E::Duplicate(_) => "alreadyExists",
            E::NotFound(_) => "notFound",
            E::Secret(_) | E::MissingSecret(_) => "secret",
            _ => "config",
        };
        Self::new(code, e.to_string())
    }
}
