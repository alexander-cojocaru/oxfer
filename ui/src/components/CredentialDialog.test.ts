import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import CredentialDialog from './CredentialDialog.svelte';
import { api } from '../lib/api';

vi.mock('../lib/api', () => ({
  api: { saveCredential: vi.fn() },
  errorMessage: (e: { message?: string }) => e?.message ?? String(e),
}));

const mocked = vi.mocked(api);

beforeEach(() => {
  vi.clearAllMocks();
  mocked.saveCredential.mockImplementation(async (_o, c) => c);
});

describe('CredentialDialog', () => {
  it('creates a password credential', async () => {
    const user = userEvent.setup();
    const onsaved = vi.fn();
    const onclose = vi.fn();
    render(CredentialDialog, { props: { onsaved, onclose } });
    expect(screen.getByRole('dialog', { name: 'New credential' })).toBeInTheDocument();
    await user.type(screen.getByLabelText('Name'), ' deploy ');
    await user.type(screen.getByLabelText('User'), 'ci');
    await user.type(screen.getByLabelText('Password'), 'pw');
    await user.click(screen.getByRole('button', { name: 'Save' }));
    expect(mocked.saveCredential).toHaveBeenCalledWith(
      null,
      { name: 'deploy', user: 'ci', auth: { method: 'password' } },
      'pw',
    );
    expect(onsaved).toHaveBeenCalled();
    expect(onclose).toHaveBeenCalled();
  });

  it('edits a key credential without sending a password', async () => {
    const user = userEvent.setup();
    const credential = { name: 'k', user: null, auth: { method: 'keyFile' as const, path: '~/.ssh/a' } };
    render(CredentialDialog, { props: { credential, onclose: vi.fn() } });
    const key = screen.getByLabelText('Key file');
    expect(key).toHaveValue('~/.ssh/a');
    await user.clear(key);
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
    await user.type(key, '~/.ssh/b');
    await user.click(screen.getByRole('button', { name: 'Save' }));
    expect(mocked.saveCredential).toHaveBeenCalledWith(
      'k',
      { name: 'k', user: null, auth: { method: 'keyFile', path: '~/.ssh/b' } },
      null,
    );
  });

  it('shows backend errors', async () => {
    const user = userEvent.setup();
    mocked.saveCredential.mockRejectedValue({ code: 'alreadyExists', message: "credential 'x' already exists" });
    const onclose = vi.fn();
    render(CredentialDialog, { props: { onclose } });
    await user.type(screen.getByLabelText('Name'), 'x');
    await user.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('already exists');
    expect(onclose).not.toHaveBeenCalled();
  });
});
