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
    await user.type(screen.getByLabelText('Group'), '  Work ');
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
        group: 'Work',
        credential: null,
      },
      null,
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
      group: 'ACME',
      credential: null,
    };
    render(ConnectionDialog, { props: { mode: 'profile', profile, onclose: vi.fn() } });
    expect(screen.getByLabelText('Host')).toHaveValue('h');
    expect(screen.getByLabelText('Group')).toHaveValue('ACME');
    const name = screen.getByLabelText('Profile name');
    await user.clear(name);
    await user.type(name, 'new');
    await user.type(screen.getByLabelText('Password'), 'pw');
    await user.click(screen.getByRole('button', { name: 'Save' }));
    expect(mocked.saveProfile).toHaveBeenCalledWith('old', { ...profile, name: 'new' }, 'pw', null);
  });

  it('a chosen credential replaces user and login fields', async () => {
    const user = userEvent.setup();
    mocked.connect.mockResolvedValue(session);
    mocked.saveProfile.mockImplementation(async (_o, p) => p);
    const credentials = [{ name: 'deploy', user: 'ci', auth: { method: 'password' as const } }];
    render(ConnectionDialog, { props: { mode: 'quick', credentials, onclose: vi.fn() } });
    await user.type(screen.getByLabelText('Host'), 'example.com');
    await user.selectOptions(screen.getByLabelText('Credential'), 'deploy');
    expect(screen.queryByLabelText('User')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Password')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Connect' }));
    expect(mocked.connect.mock.calls[0][0]).toMatchObject({ kind: 'quick', host: 'example.com', credential: 'deploy' });
  });

  it('saves a profile that uses a credential', async () => {
    const user = userEvent.setup();
    mocked.saveProfile.mockImplementation(async (_o, p) => p);
    const credentials = [{ name: 'deploy', user: 'ci', auth: { method: 'password' as const } }];
    render(ConnectionDialog, { props: { mode: 'profile', credentials, onclose: vi.fn() } });
    await user.type(screen.getByLabelText('Profile name'), 'web');
    await user.type(screen.getByLabelText('Host'), 'web.example.com');
    await user.selectOptions(screen.getByLabelText('Credential'), 'deploy');
    await user.click(screen.getByRole('button', { name: 'Save' }));
    expect(mocked.saveProfile).toHaveBeenCalledWith(
      null,
      expect.objectContaining({ name: 'web', user: null, auth: { method: 'agent' }, credential: 'deploy' }),
      null,
      null,
    );
  });

  it('duplicates a profile: prefilled copy, editable name and host, password copied server side', async () => {
    const user = userEvent.setup();
    mocked.saveProfile.mockImplementation(async (_o, p) => p);
    const profile = {
      name: 'web',
      protocol: 'sftp' as const,
      host: '10.0.0.1',
      port: 2222,
      user: 'u',
      auth: { method: 'password' as const },
      remoteDir: '/srv',
      group: 'ACME',
      credential: null,
    };
    render(ConnectionDialog, { props: { mode: 'profile', profile, duplicate: true, onclose: vi.fn() } });
    expect(screen.getByLabelText('Profile name')).toHaveValue('web copy');
    expect(screen.getByLabelText('Host')).toHaveValue('10.0.0.1');
    const host = screen.getByLabelText('Host');
    await user.clear(host);
    await user.type(host, '10.0.0.2');
    await user.click(screen.getByRole('button', { name: 'Save' }));
    expect(mocked.saveProfile).toHaveBeenCalledWith(null, { ...profile, name: 'web copy', host: '10.0.0.2' }, null, 'web');
  });
});

describe('ConnectionDialog with VNC', () => {
  it('a quick VNC connection opens a screen instead of a file session', async () => {
    const user = userEvent.setup();
    const onvnc = vi.fn();
    const onconnected = vi.fn();
    const onclose = vi.fn();
    render(ConnectionDialog, { props: { mode: 'quick', onvnc, onconnected, onclose } });

    await user.selectOptions(screen.getByLabelText('Protocol'), 'vnc');
    expect(screen.getByLabelText('Port')).toHaveAttribute('placeholder', '5900');
    // A screen has no start directory.
    expect(screen.queryByLabelText('Start directory')).not.toBeInTheDocument();
    expect(screen.getByText(/macOS Screen Sharing/)).toBeInTheDocument();

    await user.type(screen.getByLabelText('Host'), '10.0.0.5');
    await user.type(screen.getByLabelText('User'), 'alice');
    await user.type(screen.getByLabelText('Password'), 'pw');
    await user.click(screen.getByRole('button', { name: 'Connect' }));

    expect(onvnc).toHaveBeenCalledWith(
      { kind: 'quick', protocol: 'vnc', host: '10.0.0.5', port: null, user: 'alice', auth: 'password', password: 'pw' },
      'alice@10.0.0.5',
    );
    expect(mocked.connect).not.toHaveBeenCalled();
    expect(onconnected).not.toHaveBeenCalled();
    expect(onclose).toHaveBeenCalled();
  });

  it('works without a user or password (asked for later)', async () => {
    const user = userEvent.setup();
    const onvnc = vi.fn();
    render(ConnectionDialog, { props: { mode: 'quick', onvnc, onclose: vi.fn() } });
    await user.selectOptions(screen.getByLabelText('Protocol'), 'vnc');
    await user.type(screen.getByLabelText('Host'), 'desk.local');
    await user.click(screen.getByRole('button', { name: 'Connect' }));
    expect(onvnc).toHaveBeenCalledWith(
      { kind: 'quick', protocol: 'vnc', host: 'desk.local', port: null, user: null, auth: 'password', password: null },
      'desk.local',
    );
  });

  it('saving a VNC profile while connecting opens the screen by profile name', async () => {
    const user = userEvent.setup();
    mocked.saveProfile.mockImplementation(async (_o, p) => p);
    const onvnc = vi.fn();
    const onsaved = vi.fn();
    render(ConnectionDialog, { props: { mode: 'quick', onvnc, onsaved, onclose: vi.fn() } });
    await user.selectOptions(screen.getByLabelText('Protocol'), 'vnc');
    await user.type(screen.getByLabelText('Host'), 'desk.local');
    await user.click(screen.getByLabelText('Save as profile'));
    await user.type(screen.getByLabelText('Profile name'), 'Desk');
    await user.click(screen.getByRole('button', { name: 'Connect' }));

    expect(mocked.saveProfile).toHaveBeenCalledWith(
      null,
      expect.objectContaining({ name: 'Desk', protocol: 'vnc', host: 'desk.local', remoteDir: null }),
      null,
      null,
    );
    expect(onsaved).toHaveBeenCalled();
    expect(onvnc).toHaveBeenCalledWith({ kind: 'profile', name: 'Desk' }, 'Desk');
    expect(mocked.connect).not.toHaveBeenCalled();
  });
});
