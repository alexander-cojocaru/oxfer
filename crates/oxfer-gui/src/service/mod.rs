//! UI-independent session, file and transfer service. Contains no Tauri
//! types: interactive questions go through [`Prompter`], progress through
//! [`EventSink`], so everything here can be tested headlessly.

mod connect;
mod dto;
mod error;
mod manager;
mod profiles;
mod prompt;
mod transfers;

pub use connect::{ConnectEnv, ConnectRequest, QuickAuth, QuickConnect};
pub use dto::{EntryDto, EntryKindDto, ProtocolDto, SessionInfo};
pub use error::{ServiceError, ServiceResult};
pub use manager::{SessionManager, LOCAL_SESSION};
pub use profiles::{AuthDto, ProfileDto, ProfileStore};
pub use prompt::{
    ConflictChoice, HostKeyChoice, Prompt, PromptAnswer, PromptBroker, PromptEvent, Prompter,
    ScriptedPrompter, SecretKind, SecretString,
};
pub use transfers::{
    EventSink, OverwriteDto, SkippedDto, TransferEvent, TransferOptionsDto, TransferRequest,
    PROGRESS_INTERVAL,
};
