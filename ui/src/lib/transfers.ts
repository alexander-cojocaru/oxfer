// Pure reducer turning backend `transfer-event`s into queue state.

import type { SessionId, TransferEvent } from './types';

export type JobStatus = 'running' | 'retrying' | 'done' | 'skipped' | 'failed';

export interface JobView {
  jobId: number;
  src: string;
  dst: string;
  size: number;
  done: number;
  status: JobStatus;
  message?: string;
}

export type TransferStatus = 'planning' | 'running' | 'paused' | 'done' | 'failed' | 'cancelled';

export interface LogItem {
  kind: 'skipped' | 'failed';
  path: string;
  message: string;
}

export interface TransferView {
  id: number;
  srcSession: SessionId;
  dstSession: SessionId;
  sources: string[];
  destDir: string;
  retryOf: number | null;
  status: TransferStatus;
  paused: boolean;
  files: number;
  bytes: number;
  completedFiles: number;
  skippedFiles: number;
  failedFiles: number;
  /** Bytes copied (including partial files). */
  bytesDone: number;
  /** Bytes of files that were skipped (count towards overall progress). */
  bytesSkipped: number;
  /** Files currently in flight. */
  active: Record<number, JobView>;
  log: LogItem[];
  startedAt: number;
  finishedAt: number | null;
  /** Smoothed bytes per second. */
  speed: number;
  sampleAt: number;
  sampleBytes: number;
  /** Per-job sizes (for skipped/failed accounting). */
  sizes: Record<number, number>;
  /** Per-job bytes reported so far. */
  progress: Record<number, number>;
}

export interface QueueState {
  /** Newest first. */
  order: number[];
  byId: Record<number, TransferView>;
}

export const emptyQueue: QueueState = { order: [], byId: {} };

const SPEED_WINDOW_MS = 500;
const SPEED_SMOOTHING = 0.3;

function blank(id: number, now: number): TransferView {
  return {
    id,
    srcSession: '',
    dstSession: '',
    sources: [],
    destDir: '',
    retryOf: null,
    status: 'planning',
    paused: false,
    files: 0,
    bytes: 0,
    completedFiles: 0,
    skippedFiles: 0,
    failedFiles: 0,
    bytesDone: 0,
    bytesSkipped: 0,
    active: {},
    log: [],
    startedAt: now,
    finishedAt: null,
    speed: 0,
    sampleAt: now,
    sampleBytes: 0,
    sizes: {},
    progress: {},
  };
}

function setProgress(t: TransferView, jobId: number, done: number): TransferView {
  const prev = t.progress[jobId] ?? 0;
  return { ...t, bytesDone: t.bytesDone + (done - prev), progress: { ...t.progress, [jobId]: done } };
}

function sampleSpeed(t: TransferView, now: number): TransferView {
  const dt = now - t.sampleAt;
  if (dt < SPEED_WINDOW_MS) return t;
  const inst = ((t.bytesDone - t.sampleBytes) * 1000) / dt;
  const speed = t.speed === 0 ? inst : t.speed + SPEED_SMOOTHING * (inst - t.speed);
  return { ...t, speed: Math.max(0, speed), sampleAt: now, sampleBytes: t.bytesDone };
}

function withoutActive(t: TransferView, jobId: number): Record<number, JobView> {
  const { [jobId]: _gone, ...rest } = t.active;
  return rest;
}

function step(t: TransferView, ev: TransferEvent, now: number): TransferView {
  switch (ev.type) {
    case 'queued':
      return {
        ...t,
        srcSession: ev.srcSession,
        dstSession: ev.dstSession,
        sources: ev.sources,
        destDir: ev.destDir,
        retryOf: ev.retryOf,
      };
    case 'planned':
      return {
        ...t,
        status: t.paused ? 'paused' : 'running',
        files: ev.files,
        bytes: ev.bytes,
        log: [...t.log, ...ev.skipped.map((s) => ({ kind: 'skipped' as const, path: s.path, message: s.reason }))],
        sampleAt: now,
      };
    case 'started':
      return {
        ...t,
        sizes: { ...t.sizes, [ev.jobId]: ev.size },
        active: {
          ...t.active,
          [ev.jobId]: { jobId: ev.jobId, src: ev.src, dst: ev.dst, size: ev.size, done: 0, status: 'running' },
        },
      };
    case 'progress': {
      let next = setProgress(t, ev.jobId, ev.done);
      const job = next.active[ev.jobId];
      if (job) next = { ...next, active: { ...next.active, [ev.jobId]: { ...job, done: ev.done, status: 'running' } } };
      return sampleSpeed(next, now);
    }
    case 'retrying': {
      const job = t.active[ev.jobId];
      if (!job) return t;
      const next = setProgress(t, ev.jobId, 0);
      return {
        ...next,
        active: {
          ...next.active,
          [ev.jobId]: { ...job, done: 0, status: 'retrying', message: `retry ${ev.attempt}: ${ev.error}` },
        },
      };
    }
    case 'finished': {
      const next = setProgress(t, ev.jobId, ev.bytes);
      return sampleSpeed(
        { ...next, completedFiles: next.completedFiles + 1, active: withoutActive(next, ev.jobId) },
        now,
      );
    }
    case 'skipped': {
      const next = setProgress(t, ev.jobId, 0);
      return {
        ...next,
        skippedFiles: next.skippedFiles + 1,
        bytesSkipped: next.bytesSkipped + (next.sizes[ev.jobId] ?? 0),
        active: withoutActive(next, ev.jobId),
        log: [...next.log, { kind: 'skipped', path: ev.path, message: ev.reason }],
      };
    }
    case 'failed': {
      const next = ev.jobId === null ? t : setProgress(t, ev.jobId, 0);
      return {
        ...next,
        failedFiles: next.failedFiles + 1,
        active: ev.jobId === null ? next.active : withoutActive(next, ev.jobId),
        log: [...next.log, { kind: 'failed', path: ev.path, message: ev.error }],
      };
    }
    case 'paused':
      return t.finishedAt ? t : { ...t, paused: true, status: 'paused', speed: 0 };
    case 'resumed':
      return t.finishedAt
        ? t
        : { ...t, paused: false, status: t.files || t.bytes ? 'running' : 'planning', sampleAt: now, sampleBytes: t.bytesDone };
    case 'done':
      return {
        ...t,
        status: ev.cancelled ? 'cancelled' : ev.failed > 0 ? 'failed' : 'done',
        paused: false,
        completedFiles: ev.completed,
        skippedFiles: ev.skipped,
        failedFiles: ev.failed,
        active: {},
        finishedAt: now,
        speed: 0,
      };
  }
}

export function reduceTransfers(state: QueueState, ev: TransferEvent, now: number = Date.now()): QueueState {
  const id = ev.transferId;
  const existing = state.byId[id];
  const base = existing ?? blank(id, now);
  const next = step(base, ev, now);
  return {
    order: existing ? state.order : [id, ...state.order],
    byId: { ...state.byId, [id]: next },
  };
}

export function removeFinished(state: QueueState): QueueState {
  const order = state.order.filter((id) => !state.byId[id].finishedAt);
  const byId: Record<number, TransferView> = {};
  for (const id of order) byId[id] = state.byId[id];
  return { order, byId };
}

// ---- derived values ----

export function fraction(t: TransferView): number {
  if (t.finishedAt && t.status === 'done') return 1;
  if (t.bytes <= 0) return t.files > 0 ? (t.completedFiles + t.skippedFiles) / t.files : 0;
  return Math.min(1, (t.bytesDone + t.bytesSkipped) / t.bytes);
}

export function etaSeconds(t: TransferView): number | null {
  if (t.finishedAt || t.paused || t.speed <= 0) return null;
  const left = t.bytes - t.bytesDone - t.bytesSkipped;
  return left <= 0 ? 0 : left / t.speed;
}

export function isActive(t: TransferView): boolean {
  return t.finishedAt === null;
}

export function canRetry(t: TransferView): boolean {
  return t.finishedAt !== null && (t.status === 'failed' || t.status === 'cancelled');
}

export function totalSpeed(state: QueueState): number {
  return state.order.reduce((acc, id) => acc + (isActive(state.byId[id]) ? state.byId[id].speed : 0), 0);
}
