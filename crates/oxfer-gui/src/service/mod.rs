//! UI-independent session, file and transfer service. Contains no Tauri
//! types: interactive questions go through [`Prompter`], progress through
//! [`EventSink`], so everything here can be tested headlessly.

mod connect;
mod dto;
mod editor;
mod error;
mod manager;
mod profiles;
mod prompt;
mod terminal;
mod transfers;

pub use connect::{ConnectEnv, ConnectRequest, QuickAuth, QuickConnect};
pub use dto::{EntryDto, EntryKindDto, ProtocolDto, SessionInfo};
pub use editor::{system_opener, FilePreview, Opener, EDIT_LIMIT, PREVIEW_LIMIT};
pub use error::{ServiceError, ServiceResult};
pub use manager::{SessionManager, LOCAL_SESSION};
pub use profiles::{AuthDto, CredentialDto, ProfileDto, ProfileStore};
pub use prompt::{
    ConflictChoice, HostKeyChoice, Prompt, PromptAnswer, PromptBroker, PromptEvent, Prompter,
    ScriptedPrompter, SecretKind, SecretString,
};
pub use terminal::{TerminalEvent, TerminalSink};
pub use transfers::{
    EventSink, OverwriteDto, SkippedDto, TransferEvent, TransferOptionsDto, TransferRequest,
    PROGRESS_INTERVAL,
};
