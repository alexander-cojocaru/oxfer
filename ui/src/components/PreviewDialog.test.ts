import { fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import PreviewDialog from './PreviewDialog.svelte';
import { api } from '../lib/api';
import type { Entry } from '../lib/types';

vi.mock('../lib/api', () => ({
  api: { readPreview: vi.fn() },
  errorMessage: (e: { message?: string }) => e?.message ?? String(e),
}));

const entry: Entry = { name: 'a.bin', path: '/srv/a.bin', kind: 'file', size: 4096, modified: 0, permissions: 0o644 };
const props = () => ({ session: 's1', entry, oneditor: vi.fn(), onclose: vi.fn() });

beforeEach(() => vi.clearAllMocks());

describe('PreviewDialog', () => {
  it('explains binary files', async () => {
    vi.mocked(api.readPreview).mockResolvedValue({ content: null, binary: true, truncated: false, size: 4096 });
    render(PreviewDialog, { props: props() });
    expect(await screen.findByText(/binary file/)).toBeInTheDocument();
    expect(api.readPreview).toHaveBeenCalledWith('s1', '/srv/a.bin');
  });

  it('notes truncation', async () => {
    vi.mocked(api.readPreview).mockResolvedValue({ content: 'abc', binary: false, truncated: true, size: 9e9 });
    render(PreviewDialog, { props: props() });
    expect(await screen.findByRole('note')).toHaveTextContent('Only the first');
  });

  it('shows errors', async () => {
    vi.mocked(api.readPreview).mockRejectedValue({ code: 'permissionDenied', message: 'permission denied: /srv/a.bin' });
    render(PreviewDialog, { props: props() });
    expect(await screen.findByRole('alert')).toHaveTextContent('permission denied');
  });

  it('copies the text to the clipboard', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    vi.mocked(api.readPreview).mockResolvedValue({ content: 'hello\nworld', binary: false, truncated: false, size: 11 });
    render(PreviewDialog, { props: props() });
    await fireEvent.click(await screen.findByRole('button', { name: 'Copy to clipboard' }));
    expect(writeText).toHaveBeenCalledWith('hello\nworld');
  });

  it('offers no copy button for binary files', async () => {
    vi.mocked(api.readPreview).mockResolvedValue({ content: null, binary: true, truncated: false, size: 4096 });
    render(PreviewDialog, { props: props() });
    await screen.findByText(/binary file/);
    expect(screen.queryByRole('button', { name: 'Copy to clipboard' })).toBeNull();
  });
});
