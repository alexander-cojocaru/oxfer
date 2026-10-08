// Global reactive application state (Svelte 5 runes).

import { api, errorMessage } from './api';
import { emptyQueue, reduceTransfers, removeFinished, type QueueState } from './transfers';
import type { Credential, OverwritePolicy, Profile, Prompt, PromptAnswer, SessionInfo, TransferEvent } from './types';

export type ToastKind = 'info' | 'success' | 'error';

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

export interface OpenPrompt {
  id: number;
  prompt: Prompt;
}

let toastSeq = 1;

class AppState {
  sessions = $state<SessionInfo[]>([]);
  profiles = $state<Profile[]>([]);
  credentials = $state<Credential[]>([]);
  queue = $state<QueueState>(emptyQueue);
  prompts = $state<OpenPrompt[]>([]);
  toasts = $state<Toast[]>([]);
  overwrite = $state<OverwritePolicy>('ask');
  status = $state('');

  toast(kind: ToastKind, message: string, timeoutMs = kind === 'error' ? 9000 : 4000): void {
    const id = toastSeq++;
    this.toasts = [...this.toasts, { id, kind, message }];
    if (kind !== 'error') this.status = message;
    if (timeoutMs > 0) setTimeout(() => this.dismissToast(id), timeoutMs);
  }

  error(e: unknown): void {
    this.toast('error', errorMessage(e));
  }

  dismissToast(id: number): void {
    this.toasts = this.toasts.filter((t) => t.id !== id);
  }

  async loadSessions(): Promise<void> {
    try {
      this.sessions = await api.listSessions();
    } catch (e) {
      this.error(e);
    }
  }

  /** Profiles and credentials (renaming a credential changes profiles too). */
  async loadProfiles(): Promise<void> {
    try {
      [this.profiles, this.credentials] = await Promise.all([api.listProfiles(), api.listCredentials()]);
    } catch (e) {
      this.error(e);
    }
  }

  applyTransferEvent(ev: TransferEvent): void {
    this.queue = reduceTransfers(this.queue, ev);
  }

  clearFinished(): void {
    this.queue = removeFinished(this.queue);
    api.clearFinishedTransfers().catch((e) => this.error(e));
  }

  openPrompt(id: number, prompt: Prompt): void {
    if (!this.prompts.some((p) => p.id === id)) this.prompts = [...this.prompts, { id, prompt }];
  }

  closePrompt(id: number): void {
    this.prompts = this.prompts.filter((p) => p.id !== id);
  }

  async answerPrompt(id: number, answer: PromptAnswer): Promise<void> {
    this.closePrompt(id);
    try {
      await api.answerPrompt(id, answer);
    } catch (e) {
      this.error(e);
    }
  }
}

export const app = new AppState();
