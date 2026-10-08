// The only module that talks to the Tauri backend. Components import `api`
// from here, so tests can replace it with `vi.mock('../lib/api')`.

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  ConnectRequest,
  Entry,
  Profile,
  PromptAnswer,
  PromptEvent,
  ServiceError,
  SessionId,
  SessionInfo,
  TransferEvent,
  TransferRequest,
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

  startTransfer: (request: TransferRequest) => invoke<number>('start_transfer', { request }),
  pauseTransfer: (id: number) => invoke<void>('pause_transfer', { id }),
  resumeTransfer: (id: number) => invoke<void>('resume_transfer', { id }),
  cancelTransfer: (id: number) => invoke<void>('cancel_transfer', { id }),
  retryTransfer: (id: number) => invoke<number>('retry_transfer', { id }),
  clearFinishedTransfers: () => invoke<void>('clear_finished_transfers'),

  listProfiles: () => invoke<Profile[]>('list_profiles'),
  saveProfile: (original: string | null, profile: Profile, password: string | null) =>
    invoke<Profile>('save_profile', { original, profile, password }),
  deleteProfile: (name: string) => invoke<void>('delete_profile', { name }),

  answerPrompt: (id: number, answer: PromptAnswer) => invoke<void>('answer_prompt', { id, answer }),

  onTransferEvent: (cb: (e: TransferEvent) => void): Promise<UnlistenFn> =>
    listen<TransferEvent>('transfer-event', (e) => cb(e.payload)),
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
