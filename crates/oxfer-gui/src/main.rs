//! Thin Tauri 2 layer: every command maps 1:1 onto [`SessionManager`];
//! transfer progress is emitted as `transfer-event`, questions as `prompt`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use oxfer_config::{Config, KeyringStore};
use oxfer_gui::service::{
    ConnectEnv, ConnectRequest, CredentialDto, EntryDto, EventSink, FilePreview, ProfileDto,
    ProfileStore, PromptAnswer, PromptBroker, PromptEvent, Prompter, SecretString, ServiceError,
    SessionInfo, SessionManager, TerminalEvent, TerminalSink, TransferEvent, TransferRequest,
};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};
use tracing_subscriber::EnvFilter;

type CmdResult<T> = Result<T, ServiceError>;

struct AppState {
    svc: Arc<SessionManager>,
    broker: Arc<PromptBroker>,
    terminals: Arc<dyn TerminalSink>,
}

/// The update found by the last `check_for_update`, waiting for `install_update`.
#[derive(Default)]
struct PendingUpdate(std::sync::Mutex<Option<Update>>);

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateInfo {
    version: String,
    notes: Option<String>,
}

/// Download progress sent to the UI while an update is installed.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
enum UpdateProgress {
    Downloading { received: u64, total: Option<u64> },
    Installing,
}

fn update_error(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::new("update", e.to_string())
}

impl AppState {
    fn prompter(&self) -> Arc<dyn Prompter> {
        self.broker.clone()
    }
}

#[tauri::command]
async fn check_for_update(
    app: AppHandle,
    pending: State<'_, PendingUpdate>,
) -> CmdResult<Option<UpdateInfo>> {
    let update = app
        .updater()
        .map_err(update_error)?
        .check()
        .await
        .map_err(update_error)?;
    let info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        notes: u.body.clone(),
    });
    *pending.0.lock().unwrap() = update;
    Ok(info)
}

/// Downloads and installs the update found by `check_for_update`; the UI then
/// calls `restart_app`. The download is verified against the configured
/// public key before anything is replaced.
#[tauri::command]
async fn install_update(
    pending: State<'_, PendingUpdate>,
    on_progress: Channel<UpdateProgress>,
) -> CmdResult<()> {
    let update = pending
        .0
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| update_error("no update available, check for updates first"))?;
    let mut received = 0u64;
    let progress = on_progress.clone();
    update
        .download_and_install(
            move |chunk, total| {
                received += chunk as u64;
                let _ = progress.send(UpdateProgress::Downloading { received, total });
            },
            move || {
                let _ = on_progress.send(UpdateProgress::Installing);
            },
        )
        .await
        .map_err(update_error)
}

#[tauri::command]
fn restart_app(app: AppHandle) {
    app.restart()
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

// Async so it runs on the Tokio runtime: closing the session's terminals spawns tasks.
#[tauri::command]
async fn disconnect(state: State<'_, AppState>, session: String) -> CmdResult<()> {
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
async fn read_preview(
    state: State<'_, AppState>,
    session: String,
    path: String,
) -> CmdResult<FilePreview> {
    state.svc.read_preview(&session, &path).await
}

#[tauri::command]
async fn open_in_editor(
    state: State<'_, AppState>,
    session: String,
    path: String,
) -> CmdResult<String> {
    state
        .svc
        .open_in_editor(&session, &path, state.prompter())
        .await
}

#[tauri::command]
fn list_credentials(state: State<'_, AppState>) -> CmdResult<Vec<CredentialDto>> {
    state.svc.profiles().list_credentials()
}

#[tauri::command]
fn save_credential(
    state: State<'_, AppState>,
    original: Option<String>,
    credential: CredentialDto,
    password: Option<SecretString>,
) -> CmdResult<CredentialDto> {
    state
        .svc
        .profiles()
        .save_credential(original.as_deref(), credential, password)
}

#[tauri::command]
fn delete_credential(state: State<'_, AppState>, name: String) -> CmdResult<()> {
    state.svc.profiles().delete_credential(&name)
}

#[tauri::command]
async fn open_terminal(
    state: State<'_, AppState>,
    session: String,
    cols: u32,
    rows: u32,
) -> CmdResult<u64> {
    state
        .svc
        .open_terminal(&session, cols, rows, state.terminals.clone())
        .await
}

#[tauri::command]
async fn terminal_write(state: State<'_, AppState>, id: u64, data: Vec<u8>) -> CmdResult<()> {
    state.svc.terminal_write(id, &data).await
}

#[tauri::command]
async fn terminal_resize(
    state: State<'_, AppState>,
    id: u64,
    cols: u32,
    rows: u32,
) -> CmdResult<()> {
    state.svc.terminal_resize(id, cols, rows).await
}

// Async so it runs on the Tokio runtime; sync commands run on the main thread,
// where `tokio::spawn` panics and aborts the app.
#[tauri::command]
async fn close_terminal(state: State<'_, AppState>, id: u64) -> CmdResult<()> {
    state.svc.close_terminal(id)
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
    let terminal_events = app.handle().clone();
    let terminals: Arc<dyn TerminalSink> = Arc::new(move |ev: TerminalEvent| {
        if let Err(e) = terminal_events.emit("terminal-event", &ev) {
            tracing::warn!("cannot emit terminal event: {e}");
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
        terminals,
    });
    Ok(())
}

fn main() {
    init_tracing();
    tracing::info!("oxfer-gui starting");
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(PendingUpdate::default())
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
            read_preview,
            open_in_editor,
            list_credentials,
            save_credential,
            delete_credential,
            open_terminal,
            terminal_write,
            terminal_resize,
            close_terminal,
            answer_prompt,
            check_for_update,
            install_update,
            restart_app,
        ])
        .run(tauri::generate_context!());
    if let Err(e) = result {
        tracing::error!("oxfer-gui failed: {e}");
        std::process::exit(1);
    }
}
