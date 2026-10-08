import { render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ConnectionDialog from './ConnectionDialog.svelte';
import { api } from '../lib/api';

vi.mock('../lib/api', () => ({
  api: {
    connect: vi.fn(),
    saveProfile: vi.fn(),
  },
  errorMessage: (e: { message?: string }) => e?.message ?? String(e),
}));

const mocked = vi.mocked(api);
const session = { id: 's1', label: 'bob@example.com', protocol: 'sftp' as const, home: '/home/bob' };

beforeEach(() => {
  vi.clearAllMocks();
});

describe('ConnectionDialog', () => {
  it('quick connect sends the form as a quick request', async () => {
    const user = userEvent.setup();
    mocked.connect.mockResolvedValue(session);
    const onconnected = vi.fn();
    const onclose = vi.fn();
    render(ConnectionDialog, { props: { mode: 'quick', onconnected, onclose } });

    expect(screen.getByRole('dialog', { name: 'Quick connect' })).toBeInTheDocument();
    const connect = screen.getByRole('button', { name: 'Connect' });
    expect(connect).toBeDisabled();

    await user.type(screen.getByLabelText('Host'), 'example.com');
    await user.type(screen.getByLabelText('Port'), '2222');
    await user.type(screen.getByLabelText('User'), 'bob');
    await user.type(screen.getByLabelText('Password'), 'secret');
    await user.click(connect);

    expect(mocked.connect).toHaveBeenCalledWith({
      kind: 'quick',
      protocol: 'sftp',
      host: 'example.com',
      port: 2222,
      user: 'bob',
      auth: 'auto',
      password: 'secret',
      keyPath: null,
      remoteDir: null,
      insecureTls: false,
    });
    expect(onconnected).toHaveBeenCalledWith(session);
    expect(onclose).toHaveBeenCalled();
    expect(mocked.saveProfile).not.toHaveBeenCalled();
  });

  it('shows backend errors and warns about plain FTP', async () => {
    const user = userEvent.setup();
    mocked.connect.mockRejectedValue({ code: 'auth', message: 'server rejected credentials' });
    const onclose = vi.fn();
    render(ConnectionDialog, { props: { mode: 'quick', onclose } });
    await user.selectOptions(screen.getByLabelText('Protocol'), 'ftp');
    expect(screen.getByRole('note')).toHaveTextContent(/unencrypted/i);
    await user.type(screen.getByLabelText('Host'), 'ftp.example.com');
    await user.click(screen.getByRole('button', { name: 'Connect' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('server rejected credentials');
    expect(onclose).not.toHaveBeenCalled();
    expect(mocked.connect.mock.calls[0][0]).toMatchObject({ protocol: 'ftp', auth: 'password' });
  });

  it('creates a profile with key file auth; password never sent for key auth', async () => {
    const user = userEvent.setup();
    mocked.saveProfile.mockImplementation(async (_o, p) => p);
    const onsaved = vi.fn();
    render(ConnectionDialog, { props: { mode: 'profile', onsaved, onclose: vi.fn() } });
    await user.type(screen.getByLabelText('Profile name'), 'web');
    await user.type(screen.getByLabelText('Host'), 'web.example.com');
    await user.selectOptions(screen.getByLabelText('Authentication'), 'keyFile');
    const save = screen.getByRole('button', { name: 'Save' });
    expect(save).toBeDisabled(); // key path required
    await user.type(screen.getByLabelText('Key file'), '~/.ssh/id_ed25519');
    await user.click(save);
    expect(mocked.saveProfile).toHaveBeenCalledWith(
      null,
      {
        name: 'web',
        protocol: 'sftp',
        host: 'web.example.com',
        port: null,
        user: null,
        auth: { method: 'keyFile', path: '~/.ssh/id_ed25519' },
        remoteDir: null,
      },
      null,
    );
    expect(onsaved).toHaveBeenCalled();
    expect(mocked.connect).not.toHaveBeenCalled();
  });

  it('edits an existing profile and stores a new password', async () => {
    const user = userEvent.setup();
    mocked.saveProfile.mockImplementation(async (_o, p) => p);
    const profile = {
      name: 'old',
      protocol: 'ftpsExplicit' as const,
      host: 'h',
      port: 2121,
      user: 'u',
      auth: { method: 'password' as const },
      remoteDir: '/pub',
    };
    render(ConnectionDialog, { props: { mode: 'profile', profile, onclose: vi.fn() } });
    expect(screen.getByLabelText('Host')).toHaveValue('h');
    const name = screen.getByLabelText('Profile name');
    await user.clear(name);
    await user.type(name, 'new');
    await user.type(screen.getByLabelText('Password'), 'pw');
    await user.click(screen.getByRole('button', { name: 'Save' }));
    expect(mocked.saveProfile).toHaveBeenCalledWith('old', { ...profile, name: 'new' }, 'pw');
  });
});
