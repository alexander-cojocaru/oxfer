import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import App from './App.svelte';
import { api } from './lib/api';
import type { PromptEvent, TransferEvent } from './lib/types';

let transferCb: ((e: TransferEvent) => void) | undefined;
let promptCb: ((e: PromptEvent) => void) | undefined;

vi.mock('./lib/api', () => ({
  api: {
    listSessions: vi.fn().mockResolvedValue([{ id: 'local', label: 'Local', protocol: 'local', home: '/home/u' }]),
    listProfiles: vi.fn().mockResolvedValue([]),
    listCredentials: vi.fn().mockResolvedValue([]),
    listDir: vi.fn().mockImplementation(async (_s: string, path: string) => [
      { name: 'docs', path: `${path}/docs`, kind: 'dir', size: 0, modified: 0, permissions: 0o755 },
      { name: 'notes.txt', path: `${path}/notes.txt`, kind: 'file', size: 12, modified: 0, permissions: 0o644 },
    ]),
    startTransfer: vi.fn().mockResolvedValue(1),
    answerPrompt: vi.fn().mockResolvedValue(undefined),
    clearFinishedTransfers: vi.fn().mockResolvedValue(undefined),
    onTransferEvent: vi.fn(async (cb) => {
      transferCb = cb;
      return () => {};
    }),
    onPrompt: vi.fn(async (cb) => {
      promptCb = cb;
      return () => {};
    }),
  },
  errorMessage: (e: unknown) => String(e),
}));

describe('App (smoke)', () => {
  it('mounts both panes, reacts to events and prompts', async () => {
    render(App);
    await waitFor(() => expect(screen.getAllByText('notes.txt')).toHaveLength(2));
    expect(screen.getAllByRole('grid')).toHaveLength(2);

    // same folder on both sides: F5 is refused in the UI
    await fireEvent.click(screen.getAllByText('notes.txt')[0]);
    await fireEvent.keyDown(window, { key: 'F5' });
    expect(api.startTransfer).not.toHaveBeenCalled();
    expect((await screen.findAllByText(/source and destination are the same/)).length).toBeGreaterThan(0);

    // open "docs" in the right pane, then F5 copies the left selection there
    await fireEvent.dblClick(screen.getAllByText('docs')[1]);
    await waitFor(() => expect(api.listDir).toHaveBeenCalledWith('local', '/home/u/docs'));
    await fireEvent.click(screen.getAllByText('notes.txt')[0]);
    await fireEvent.keyDown(window, { key: 'F5' });
    expect(api.startTransfer).toHaveBeenCalledWith({
      srcSession: 'local',
      dstSession: 'local',
      sources: ['/home/u/notes.txt'],
      destDir: '/home/u/docs',
      options: { overwrite: 'ask' },
    });
  });

  it('shows host key prompts from the backend', async () => {
    render(App);
    await waitFor(() => expect(promptCb).toBeDefined());
    promptCb!({
      type: 'open',
      id: 9,
      prompt: { kind: 'hostKey', host: 'h', port: 22, algorithm: 'ssh-ed25519', fingerprint: 'SHA256:zzz' },
    });
    expect(await screen.findByText('SHA256:zzz')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Accept once' }));
    expect(api.answerPrompt).toHaveBeenCalledWith(9, { kind: 'hostKey', decision: 'acceptOnce' });

    transferCb!({
      type: 'queued',
      transferId: 5,
      srcSession: 'local',
      dstSession: 'local',
      sources: ['/x/a.bin'],
      destDir: '/y',
      retryOf: null,
    });
    expect(await screen.findByText('a.bin → /y')).toBeInTheDocument();
  });
});
