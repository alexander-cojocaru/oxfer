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
    connect: vi.fn().mockResolvedValue({ id: 's1', label: 'web', protocol: 'sftp', home: '/srv' }),
    readPreview: vi.fn().mockResolvedValue({ content: 'hello world\nline 2', binary: false, truncated: false, size: 18 }),
    openInEditor: vi.fn().mockResolvedValue('/home/u/notes.txt'),
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

  it('double-clicking a file previews it; F4 opens it in the editor', async () => {
    render(App);
    await waitFor(() => expect(screen.getAllByText('notes.txt')).toHaveLength(2));
    await fireEvent.dblClick(screen.getAllByText('notes.txt')[0]);
    const dialog = await screen.findByRole('dialog', { name: 'notes.txt' });
    expect(api.readPreview).toHaveBeenCalledWith('local', '/home/u/notes.txt');
    expect(await screen.findByLabelText('Contents of notes.txt')).toHaveTextContent('hello world line 2');
    expect(dialog).toHaveTextContent('2 lines');
    await fireEvent.click(screen.getByRole('button', { name: 'Open in editor' }));
    expect(api.openInEditor).toHaveBeenCalledWith('local', '/home/u/notes.txt');
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'notes.txt' })).not.toBeInTheDocument());

    vi.mocked(api.openInEditor).mockClear();
    await fireEvent.click(screen.getAllByText('notes.txt')[1]);
    await fireEvent.keyDown(screen.getAllByRole('grid')[1], { key: 'F4' });
    expect(api.openInEditor).toHaveBeenCalledWith('local', '/home/u/notes.txt');
  });

  it('connecting to an open server switches to its session instead of opening another', async () => {
    vi.mocked(api.listProfiles).mockResolvedValueOnce([
      {
        name: 'web',
        protocol: 'sftp',
        host: 'web.example.com',
        port: null,
        user: 'bob',
        auth: { method: 'agent' },
        remoteDir: null,
        group: null,
        credential: null,
      },
    ]);
    render(App);
    const connectWeb = await screen.findByRole('button', { name: 'Connect web' });
    await fireEvent.click(connectWeb);
    // toast and status bar
    expect((await screen.findAllByText('Connected to web')).length).toBeGreaterThan(0);
    await fireEvent.click(connectWeb);
    expect((await screen.findAllByText(/Already connected to web/)).length).toBeGreaterThan(0);
    expect(api.connect).toHaveBeenCalledTimes(2);
    // still one entry under "Connections"
    expect(screen.getAllByRole('button', { name: 'Disconnect web' })).toHaveLength(1);
  });
});
