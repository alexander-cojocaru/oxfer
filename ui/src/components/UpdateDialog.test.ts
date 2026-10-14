import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import UpdateDialog from './UpdateDialog.svelte';
import { api } from '../lib/api';

vi.mock('../lib/api', () => ({
  api: { checkForUpdate: vi.fn(), installUpdate: vi.fn(), restartApp: vi.fn() },
  errorMessage: (e: unknown) => (e as { message?: string }).message ?? String(e),
}));

beforeEach(() => vi.clearAllMocks());

describe('UpdateDialog', () => {
  it('reports that the app is up to date', async () => {
    vi.mocked(api.checkForUpdate).mockResolvedValue(null);
    render(UpdateDialog, { onclose: () => {} });
    expect(await screen.findByText(/is up to date/)).toBeInTheDocument();
  });

  it('installs an available update and offers a restart', async () => {
    vi.mocked(api.checkForUpdate).mockResolvedValue({ version: '9.9.9', notes: '- Faster\n- Fixes' });
    vi.mocked(api.installUpdate).mockImplementation(async (cb) => {
      cb({ kind: 'downloading', received: 5, total: 10 });
      cb({ kind: 'installing' });
    });
    render(UpdateDialog, { onclose: () => {} });
    expect(await screen.findByText(/9\.9\.9 is available/)).toBeInTheDocument();
    expect(screen.getByText('Faster')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Download and install' }));
    await fireEvent.click(await screen.findByRole('button', { name: 'Restart now' }));
    expect(api.restartApp).toHaveBeenCalled();
  });

  it('shows errors and allows a retry', async () => {
    vi.mocked(api.checkForUpdate).mockRejectedValueOnce({ message: 'offline' }).mockResolvedValueOnce(null);
    render(UpdateDialog, { onclose: () => {} });
    expect(await screen.findByRole('alert')).toHaveTextContent('offline');
    await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    await waitFor(() => expect(screen.getByText(/is up to date/)).toBeInTheDocument());
  });
});
