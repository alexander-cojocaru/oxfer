import { describe, expect, it } from 'vitest';
import { canRetry, emptyQueue, etaSeconds, fraction, reduceTransfers, removeFinished, totalSpeed, type QueueState } from './transfers';
import type { TransferEvent } from './types';

function run(events: TransferEvent[], start = 1000, step = 100): QueueState {
  return events.reduce((s, ev, i) => reduceTransfers(s, ev, start + i * step), emptyQueue);
}

const queued: TransferEvent = {
  type: 'queued',
  transferId: 1,
  srcSession: 'local',
  dstSession: 's1',
  sources: ['/a/x.bin', '/a/y.bin'],
  destDir: '/remote',
  retryOf: null,
};

describe('transfer reducer', () => {
  it('tracks progress, speed, eta and completion', () => {
    let s = run([
      queued,
      { type: 'planned', transferId: 1, files: 2, bytes: 2000, skipped: [] },
      { type: 'started', transferId: 1, jobId: 0, src: '/a/x.bin', dst: '/remote/x.bin', size: 1000 },
      { type: 'progress', transferId: 1, jobId: 0, done: 500, total: 1000 },
    ]);
    let tr = s.byId[1];
    expect(tr.status).toBe('running');
    expect(tr.bytesDone).toBe(500);
    expect(tr.active[0].done).toBe(500);
    expect(fraction(tr)).toBeCloseTo(0.25);

    // 1 s later: 1000 more bytes → speed sample
    s = reduceTransfers(s, { type: 'progress', transferId: 1, jobId: 0, done: 1000, total: 1000 }, 3000);
    s = reduceTransfers(s, { type: 'finished', transferId: 1, jobId: 0, bytes: 1000 }, 3000);
    tr = s.byId[1];
    expect(tr.completedFiles).toBe(1);
    expect(tr.active[0]).toBeUndefined();
    expect(tr.speed).toBeGreaterThan(0);
    expect(etaSeconds(tr)).toBeGreaterThan(0);
    expect(totalSpeed(s)).toBe(tr.speed);

    s = reduceTransfers(s, {
      type: 'done',
      transferId: 1,
      completed: 2,
      skipped: 0,
      failed: 0,
      bytes: 2000,
      cancelled: false,
    });
    tr = s.byId[1];
    expect(tr.status).toBe('done');
    expect(fraction(tr)).toBe(1);
    expect(etaSeconds(tr)).toBeNull();
    expect(canRetry(tr)).toBe(false);
  });

  it('logs skipped and failed files and allows retry', () => {
    const s = run([
      queued,
      { type: 'planned', transferId: 1, files: 2, bytes: 30, skipped: [{ path: '/a/link', reason: 'symlink' }] },
      { type: 'started', transferId: 1, jobId: 0, src: '/a/x.bin', dst: '/remote/x.bin', size: 10 },
      { type: 'skipped', transferId: 1, jobId: 0, path: '/a/x.bin', reason: 'skipped by user' },
      { type: 'started', transferId: 1, jobId: 1, src: '/a/y.bin', dst: '/remote/y.bin', size: 20 },
      { type: 'progress', transferId: 1, jobId: 1, done: 5, total: 20 },
      { type: 'retrying', transferId: 1, jobId: 1, attempt: 1, error: 'reset' },
      { type: 'failed', transferId: 1, jobId: 1, path: '/a/y.bin', error: 'boom' },
      { type: 'done', transferId: 1, completed: 0, skipped: 2, failed: 1, bytes: 0, cancelled: false },
    ]);
    const tr = s.byId[1];
    expect(tr.status).toBe('failed');
    expect(tr.bytesSkipped).toBe(10);
    expect(tr.bytesDone).toBe(0);
    expect(tr.log.map((l) => [l.kind, l.path])).toEqual([
      ['skipped', '/a/link'],
      ['skipped', '/a/x.bin'],
      ['failed', '/a/y.bin'],
    ]);
    expect(canRetry(tr)).toBe(true);
  });

  it('handles pause/resume, cancel, ordering and clearing', () => {
    let s = run([
      queued,
      { type: 'paused', transferId: 1 },
      { ...queued, transferId: 2, retryOf: 1 },
    ]);
    expect(s.order).toEqual([2, 1]);
    expect(s.byId[1].status).toBe('paused');
    expect(s.byId[2].retryOf).toBe(1);
    s = reduceTransfers(s, { type: 'resumed', transferId: 1 });
    expect(s.byId[1].paused).toBe(false);
    s = reduceTransfers(s, { type: 'done', transferId: 1, completed: 0, skipped: 0, failed: 0, bytes: 0, cancelled: true });
    expect(s.byId[1].status).toBe('cancelled');
    expect(canRetry(s.byId[1])).toBe(true);
    // events after done do not resurrect the transfer
    s = reduceTransfers(s, { type: 'paused', transferId: 1 });
    expect(s.byId[1].status).toBe('cancelled');
    s = removeFinished(s);
    expect(s.order).toEqual([2]);
  });

  it('planning failure without job id', () => {
    const s = run([
      queued,
      { type: 'failed', transferId: 1, jobId: null, path: '/remote', error: 'not found' },
      { type: 'done', transferId: 1, completed: 0, skipped: 0, failed: 1, bytes: 0, cancelled: false },
    ]);
    expect(s.byId[1].status).toBe('failed');
    expect(s.byId[1].log[0].message).toBe('not found');
  });
});
