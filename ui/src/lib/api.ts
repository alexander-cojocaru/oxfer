// The only module that talks to the Tauri backend. Components import `api`
// from here, so tests can replace it with `vi.mock('../lib/api')`.

import { Channel, invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  ConnectRequest,
  Credential,
  Entry,
  FilePreview,
  Profile,
  PromptAnswer,
  PromptEvent,
  ServiceError,
  SessionId,
  SessionInfo,
  TerminalEvent,
  TransferEvent,
  TransferRequest,
  UpdateInfo,
  UpdateProgress,
} from './types';

export const api = {
  listSessions: () => invoke<SessionInfo[]>('list_sessions'),
  connect: (request: ConnectRequest) => invoke<SessionInfo>('connect', { request }),
  disconnect: (session: SessionId) => invoke<void>('disconnect', { session }),

  listDir: (session: SessionId, path: string) => invoke<Entry[]>('list_dir', { session, path }),
  stat: (session: SessionId, path: string) => invoke<Entry>('stat', { session, path }),
  realpath: (session: SessionId, path: string) => invoke<string>('realpath', { session, path }),
  mkdir: (session: SessionId, path: string) => invoke<void>('mkdir', { session, path }),
  rename: (session: SessionId, from: string, to: string) =>
    invoke<void>('rename', { session, from, to }),
  delete: (session: SessionId, paths: string[]) => invoke<void>('delete', { session, paths }),
  readPreview: (session: SessionId, path: string) => invoke<FilePreview>('read_preview', { session, path }),
  /** Returns the path of the file the editor opened (a local copy for remote files). */
  openInEditor: (session: SessionId, path: string) => invoke<string>('open_in_editor', { session, path }),

  startTransfer: (request: TransferRequest) => invoke<number>('start_transfer', { request }),
  pauseTransfer: (id: number) => invoke<void>('pause_transfer', { id }),
  resumeTransfer: (id: number) => invoke<void>('resume_transfer', { id }),
  cancelTransfer: (id: number) => invoke<void>('cancel_transfer', { id }),
  retryTransfer: (id: number) => invoke<number>('retry_transfer', { id }),
  clearFinishedTransfers: () => invoke<void>('clear_finished_transfers'),

  listProfiles: () => invoke<Profile[]>('list_profiles'),
  saveProfile: (original: string | null, profile: Profile, password: string | null, copyFrom: string | null = null) =>
    invoke<Profile>('save_profile', { original, profile, password, copyFrom }),
  exportProfiles: (path: string) => invoke<number>('export_profiles', { path }),
  importProfiles: (path: string, overwrite: boolean) =>
    invoke<{ added: number; replaced: number; skipped: number }>('import_profiles', { path, overwrite }),
  deleteProfile: (name: string) => invoke<void>('delete_profile', { name }),

  listCredentials: () => invoke<Credential[]>('list_credentials'),
  saveCredential: (original: string | null, credential: Credential, password: string | null) =>
    invoke<Credential>('save_credential', { original, credential, password }),
  deleteCredential: (name: string) => invoke<void>('delete_credential', { name }),

  openTerminal: (session: SessionId, cols: number, rows: number) =>
    invoke<number>('open_terminal', { session, cols, rows }),
  terminalWrite: (id: number, data: number[]) => invoke<void>('terminal_write', { id, data }),
  terminalResize: (id: number, cols: number, rows: number) =>
    invoke<void>('terminal_resize', { id, cols, rows }),
  closeTerminal: (id: number) => invoke<void>('close_terminal', { id }),

  answerPrompt: (id: number, answer: PromptAnswer) => invoke<void>('answer_prompt', { id, answer }),

  /** Resolves to `null` when the app is up to date. */
  checkForUpdate: () => invoke<UpdateInfo | null>('check_for_update'),
  /** Downloads and installs the update found by `checkForUpdate`; call `restartApp` afterwards. */
  installUpdate: (onProgress: (p: UpdateProgress) => void) => {
    const channel = new Channel<UpdateProgress>();
    channel.onmessage = onProgress;
    return invoke<void>('install_update', { onProgress: channel });
  },
  restartApp: () => invoke<void>('restart_app'),

  onTransferEvent: (cb: (e: TransferEvent) => void): Promise<UnlistenFn> =>
    listen<TransferEvent>('transfer-event', (e) => cb(e.payload)),
  onTerminalEvent: (cb: (e: TerminalEvent) => void): Promise<UnlistenFn> =>
    listen<TerminalEvent>('terminal-event', (e) => cb(e.payload)),
  onPrompt: (cb: (e: PromptEvent) => void): Promise<UnlistenFn> =>
    listen<PromptEvent>('prompt', (e) => cb(e.payload)),
};

export type Api = typeof api;

/** Normalise anything thrown by `invoke` into `{ code, message }`. */
export function toServiceError(e: unknown): ServiceError {
  if (e && typeof e === 'object' && 'code' in e && 'message' in e) {
    return { code: String(e.code), message: String(e.message) };
  }
  if (e instanceof Error) return { code: 'internal', message: e.message };
  return { code: 'internal', message: String(e) };
}

export function errorMessage(e: unknown): string {
  return toServiceError(e).message;
}
