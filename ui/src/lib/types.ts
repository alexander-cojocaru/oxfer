// Mirrors the serde DTOs in crates/oxfer-gui/src/service (camelCase).

export type SessionId = string;
export const LOCAL_SESSION: SessionId = 'local';

export type EntryKind = 'file' | 'dir' | 'symlink' | 'other';

export interface Entry {
  name: string;
  path: string;
  kind: EntryKind;
  size: number;
  /** Milliseconds since the Unix epoch. */
  modified: number | null;
  permissions: number | null;
}

export type RemoteProtocol = 'sftp' | 'ftp' | 'ftpsExplicit' | 'ftpsImplicit';
export type Protocol = 'local' | RemoteProtocol;

export interface SessionInfo {
  id: SessionId;
  label: string;
  protocol: Protocol;
  home: string;
}

export type AuthDto =
  | { method: 'agent' }
  | { method: 'keyFile'; path: string }
  | { method: 'password' }
  | { method: 'interactive' };

export interface Profile {
  name: string;
  protocol: RemoteProtocol;
  host: string;
  port: number | null;
  user: string | null;
  auth: AuthDto;
  remoteDir: string | null;
  /** Folder the profile is listed under; `null` for the top level. */
  group: string | null;
  /** Shared credential; when set, it supplies the user and authentication. */
  credential: string | null;
}

/** A user name plus authentication that several profiles can share. */
export interface Credential {
  name: string;
  user: string | null;
  auth: AuthDto;
}

export type QuickAuth = 'auto' | 'agent' | 'keyFile' | 'password';

export interface QuickConnect {
  protocol: RemoteProtocol;
  host: string;
  port?: number | null;
  user?: string | null;
  auth: QuickAuth;
  password?: string | null;
  keyPath?: string | null;
  remoteDir?: string | null;
  insecureTls?: boolean;
  /** Saved credential replacing user, auth, password and key path. */
  credential?: string | null;
}

export type ConnectRequest = { kind: 'profile'; name: string } | ({ kind: 'quick' } & QuickConnect);

export type OverwritePolicy = 'ask' | 'overwrite' | 'skip' | 'newerOnly';

export interface TransferOptions {
  overwrite: OverwritePolicy;
  resume?: boolean;
  verify?: boolean;
  concurrency?: number | null;
}

export interface TransferRequest {
  srcSession: SessionId;
  dstSession: SessionId;
  sources: string[];
  destDir: string;
  options: TransferOptions;
}

export interface SkippedItem {
  path: string;
  reason: string;
}

export type TransferEvent =
  | {
      type: 'queued';
      transferId: number;
      srcSession: SessionId;
      dstSession: SessionId;
      sources: string[];
      destDir: string;
      retryOf: number | null;
    }
  | { type: 'planned'; transferId: number; files: number; bytes: number; skipped: SkippedItem[] }
  | { type: 'started'; transferId: number; jobId: number; src: string; dst: string; size: number }
  | { type: 'progress'; transferId: number; jobId: number; done: number; total: number }
  | { type: 'retrying'; transferId: number; jobId: number; attempt: number; error: string }
  | { type: 'skipped'; transferId: number; jobId: number; path: string; reason: string }
  | { type: 'finished'; transferId: number; jobId: number; bytes: number }
  | { type: 'failed'; transferId: number; jobId: number | null; path: string; error: string }
  | { type: 'paused'; transferId: number }
  | { type: 'resumed'; transferId: number }
  | {
      type: 'done';
      transferId: number;
      completed: number;
      skipped: number;
      failed: number;
      bytes: number;
      cancelled: boolean;
    };

export type TerminalEvent =
  | { kind: 'output'; terminalId: number; data: number[] }
  | { kind: 'closed'; terminalId: number };

export type Prompt =
  | { kind: 'hostKey'; host: string; port: number; algorithm: string; fingerprint: string }
  | { kind: 'password'; user: string; host: string; secret: 'password' | 'passphrase'; retry: boolean }
  | { kind: 'confirm'; title: string; message: string; danger: boolean }
  | { kind: 'conflict'; transferId: number; src: Entry; dst: Entry };

export type PromptAnswer =
  | { kind: 'hostKey'; decision: 'acceptAndSave' | 'acceptOnce' | 'reject' }
  | { kind: 'password'; value: string }
  | { kind: 'confirm'; ok: boolean }
  | { kind: 'conflict'; action: 'overwrite' | 'skip'; applyToAll: boolean }
  | { kind: 'cancel' };

export type PromptEvent = { type: 'open'; id: number; prompt: Prompt } | { type: 'closed'; id: number };

export interface ServiceError {
  code: string;
  message: string;
}

export interface FilePreview {
  /** `null` for binary files. */
  content: string | null;
  binary: boolean;
  /** Only the first megabyte is included. */
  truncated: boolean;
  size: number;
}

export interface UpdateInfo {
  version: string;
  /** Release notes (the CHANGELOG section of the release). */
  notes: string | null;
}

export type UpdateProgress =
  | { kind: 'downloading'; received: number; total: number | null }
  | { kind: 'installing' };
