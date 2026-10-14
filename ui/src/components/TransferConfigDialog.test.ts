import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocked = vi.hoisted(() => ({ exportProfiles: vi.fn(), importProfiles: vi.fn(), loadProfiles: vi.fn(), toast: vi.fn() }));
vi.mock('../lib/api', () => ({
  api: { exportProfiles: mocked.exportProfiles, importProfiles: mocked.importProfiles },
  errorMessage: (e: unknown) => String(e),
}));
vi.mock('../lib/state.svelte', () => ({ app: { loadProfiles: mocked.loadProfiles, toast: mocked.toast } }));

import TransferConfigDialog from './TransferConfigDialog.svelte';

describe('TransferConfigDialog', () => {
  beforeEach(() => vi.clearAllMocks());

  it('exports to the typed path', async () => {
    const user = userEvent.setup();
    mocked.exportProfiles.mockResolvedValue(3);
    const onclose = vi.fn();
    render(TransferConfigDialog, { props: { mode: 'export', onclose } });
    const button = screen.getByRole('button', { name: 'Export' });
    expect(button).toBeDisabled();
    await user.type(screen.getByLabelText('File path'), '/tmp/p.toml');
    await user.click(button);
    expect(mocked.exportProfiles).toHaveBeenCalledWith('/tmp/p.toml');
    expect(onclose).toHaveBeenCalled();
  });

  it('imports with the overwrite choice and reloads profiles', async () => {
    const user = userEvent.setup();
    mocked.importProfiles.mockResolvedValue({ added: 1, replaced: 0, skipped: 2 });
    render(TransferConfigDialog, { props: { mode: 'import', onclose: vi.fn() } });
    await user.type(screen.getByLabelText('File path'), '/tmp/p.toml');
    await user.click(screen.getByLabelText(/Replace existing/));
    await user.click(screen.getByRole('button', { name: 'Import' }));
    expect(mocked.importProfiles).toHaveBeenCalledWith('/tmp/p.toml', true);
    expect(mocked.loadProfiles).toHaveBeenCalled();
  });

  it('shows an error and stays open', async () => {
    const user = userEvent.setup();
    mocked.importProfiles.mockRejectedValue('boom');
    const onclose = vi.fn();
    render(TransferConfigDialog, { props: { mode: 'import', onclose } });
    await user.type(screen.getByLabelText('File path'), '/x');
    await user.click(screen.getByRole('button', { name: 'Import' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('boom');
    expect(onclose).not.toHaveBeenCalled();
  });
});
