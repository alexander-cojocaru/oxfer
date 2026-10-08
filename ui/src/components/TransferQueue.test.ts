import { fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import TransferQueue from './TransferQueue.svelte';
import { api } from '../lib/api';
import { emptyQueue, reduceTransfers, type QueueState } from '../lib/transfers';
import type { TransferEvent } from '../lib/types';

vi.mock('../lib/api', () => ({
  api: {
    pauseTransfer: vi.fn().mockResolvedValue(undefined),
    resumeTransfer: vi.fn().mockResolvedValue(undefined),
    cancelTransfer: vi.fn().mockResolvedValue(undefined),
    retryTransfer: vi.fn().mockResolvedValue(2),
  },
  errorMessage: (e: unknown) => String(e),
}));

const mocked = vi.mocked(api);

function queue(...events: TransferEvent[]): QueueState {
  return events.reduce((s, e) => reduceTransfers(s, e, 0), emptyQueue);
}

const queued: TransferEvent = {
  type: 'queued',
  transferId: 1,
  srcSession: 'local',
  dstSession: 's1',
  sources: ['/a/report.pdf'],
  destDir: '/remote',
  retryOf: null,
};

beforeEach(() => vi.clearAllMocks());

describe('TransferQueue', () => {
  it('shows progress and controls a running transfer', async () => {
    const q = queue(
      queued,
      { type: 'planned', transferId: 1, files: 1, bytes: 100, skipped: [] },
      { type: 'started', transferId: 1, jobId: 0, src: '/a/report.pdf', dst: '/remote/report.pdf', size: 100 },
      { type: 'progress', transferId: 1, jobId: 0, done: 40, total: 100 },
    );
    const { rerender } = render(TransferQueue, { props: { queue: q } });
    expect(screen.getByText('report.pdf → /remote')).toBeInTheDocument();
    expect(screen.getAllByRole('progressbar')[0]).toHaveAttribute('aria-valuenow', '40');
    await fireEvent.click(screen.getByRole('button', { name: 'Pause' }));
    expect(mocked.pauseTransfer).toHaveBeenCalledWith(1);
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(mocked.cancelTransfer).toHaveBeenCalledWith(1);

    await rerender({ queue: reduceTransfers(q, { type: 'paused', transferId: 1 }) });
    await fireEvent.click(screen.getByRole('button', { name: 'Resume' }));
    expect(mocked.resumeTransfer).toHaveBeenCalledWith(1);
  });

  it('failed transfers can be retried and show their log', async () => {
    const q = queue(
      queued,
      { type: 'planned', transferId: 1, files: 1, bytes: 100, skipped: [] },
      { type: 'failed', transferId: 1, jobId: 0, path: '/a/report.pdf', error: 'permission denied' },
      { type: 'done', transferId: 1, completed: 0, skipped: 0, failed: 1, bytes: 0, cancelled: false },
    );
    render(TransferQueue, { props: { queue: q } });
    expect(screen.getByText('Failed')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Details' }));
    expect(screen.getByText('permission denied')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(mocked.retryTransfer).toHaveBeenCalledWith(1);
  });

  it('empty state', () => {
    render(TransferQueue, { props: { queue: emptyQueue } });
    expect(screen.getByText(/No transfers yet/)).toBeInTheDocument();
  });
});
