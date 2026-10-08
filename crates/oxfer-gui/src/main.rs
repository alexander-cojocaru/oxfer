//! Thin Tauri 2 layer: every command maps 1:1 onto [`SessionManager`];
//! transfer progress is emitted as `transfer-event`, questions as `prompt`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use oxfer_config::{Config, KeyringStore};
use oxfer_gui::service::{
    ConnectEnv, ConnectRequest, EntryDto, EventSink, ProfileDto, ProfileStore, PromptAnswer,
    PromptBroker, PromptEvent, Prompter, SecretString, ServiceError, SessionInfo, SessionManager,
    TransferEvent, TransferRequest,
};
use tauri::{Emitter, Manager, State};
use tracing_subscriber::EnvFilter;

type CmdResult<T> = Result<T, ServiceError>;

struct AppState {
    svc: Arc<SessionManager>,
    broker: Arc<PromptBroker>,
}

impl AppState {
    fn prompter(&self) -> Arc<dyn Prompter> {
        self.broker.clone()
    }
}

#[tauri::command]
fn list_sessions(state: State<'_, AppState>) -> Vec<SessionInfo> {
    tracing::debug!("frontend requested sessions");
    state.svc.sessions()
}

#[tauri::command]
async fn connect(state: State<'_, AppState>, request: ConnectRequest) -> CmdResult<SessionInfo> {
    state.svc.connect(request, state.prompter()).await
}

#[tauri::command]
fn disconnect(state: State<'_, AppState>, session: String) -> CmdResult<()> {
    state.svc.disconnect(&session)
}

#[tauri::command]
async fn list_dir(
    state: State<'_, AppState>,
    session: String,
    path: String,
) -> CmdResult<Vec<EntryDto>> {
    state.svc.list_dir(&session, &path).await
}

#[tauri::command]
async fn stat(state: State<'_, AppState>, session: String, path: String) -> CmdResult<EntryDto> {
    state.svc.stat(&session, &path).await
}

#[tauri::command]
async fn realpath(state: State<'_, AppState>, session: String, path: String) -> CmdResult<String> {
    state.svc.realpath(&session, &path).await
}

#[tauri::command]
async fn mkdir(state: State<'_, AppState>, session: String, path: String) -> CmdResult<()> {
    state.svc.mkdir(&session, &path).await
}

#[tauri::command]
async fn rename(
    state: State<'_, AppState>,
    session: String,
    from: String,
    to: String,
) -> CmdResult<()> {
    state.svc.rename(&session, &from, &to).await
}

#[tauri::command]
async fn delete(state: State<'_, AppState>, session: String, paths: Vec<String>) -> CmdResult<()> {
    state.svc.delete(&session, &paths).await
}

#[tauri::command]
async fn start_transfer(state: State<'_, AppState>, request: TransferRequest) -> CmdResult<u64> {
    state.svc.start_transfer(request, state.prompter()).await
}

#[tauri::command]
fn pause_transfer(state: State<'_, AppState>, id: u64) -> CmdResult<()> {
    state.svc.pause_transfer(id)
}

#[tauri::command]
fn resume_transfer(state: State<'_, AppState>, id: u64) -> CmdResult<()> {
    state.svc.resume_transfer(id)
}

#[tauri::command]
fn cancel_transfer(state: State<'_, AppState>, id: u64) -> CmdResult<()> {
    state.svc.cancel_transfer(id)
}

#[tauri::command]
async fn retry_transfer(state: State<'_, AppState>, id: u64) -> CmdResult<u64> {
    state.svc.retry_transfer(id, state.prompter()).await
}

#[tauri::command]
fn clear_finished_transfers(state: State<'_, AppState>) {
    state.svc.clear_finished()
}

#[tauri::command]
fn list_profiles(state: State<'_, AppState>) -> CmdResult<Vec<ProfileDto>> {
    state.svc.profiles().list()
}

#[tauri::command]
fn save_profile(
    state: State<'_, AppState>,
    original: Option<String>,
    profile: ProfileDto,
    password: Option<SecretString>,
) -> CmdResult<ProfileDto> {
    state
        .svc
        .profiles()
        .save(original.as_deref(), profile, password)
}

#[tauri::command]
fn delete_profile(state: State<'_, AppState>, name: String) -> CmdResult<()> {
    state.svc.profiles().delete(&name)
}

#[tauri::command]
fn answer_prompt(state: State<'_, AppState>, id: u64, answer: PromptAnswer) -> CmdResult<()> {
    state.broker.answer(id, answer)
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let events = app.handle().clone();
    let sink: Arc<dyn EventSink> = Arc::new(move |ev: TransferEvent| {
        if let Err(e) = events.emit("transfer-event", &ev) {
            tracing::warn!("cannot emit transfer event: {e}");
        }
    });
    let prompts = app.handle().clone();
    let broker = PromptBroker::new(move |ev: PromptEvent| {
        if let Err(e) = prompts.emit("prompt", &ev) {
            tracing::warn!("cannot emit prompt: {e}");
        }
    });
    let profiles = ProfileStore::new(Config::default_path()?, Arc::new(KeyringStore));
    let env = ConnectEnv {
        use_default_identities: true,
        ..ConnectEnv::default()
    };
    app.manage(AppState {
        svc: SessionManager::new(profiles, env, sink),
        broker,
    });
    Ok(())
}

fn main() {
    init_tracing();
    tracing::info!("oxfer-gui starting");
    let result = tauri::Builder::default()
        .setup(setup)
        .invoke_handler(tauri::generate_handler![
            list_sessions,
            connect,
            disconnect,
            list_dir,
            stat,
            realpath,
            mkdir,
            rename,
            delete,
            start_transfer,
            pause_transfer,
            resume_transfer,
            cancel_transfer,
            retry_transfer,
            clear_finished_transfers,
            list_profiles,
            save_profile,
            delete_profile,
            answer_prompt,
        ])
        .run(tauri::generate_context!());
    if let Err(e) = result {
        tracing::error!("oxfer-gui failed: {e}");
        std::process::exit(1);
    }
}
