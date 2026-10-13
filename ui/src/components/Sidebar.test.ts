import { render, screen, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Sidebar from './Sidebar.svelte';
import { api } from '../lib/api';
import { app } from '../lib/state.svelte';
import type { Profile } from '../lib/types';

vi.mock('../lib/api', () => ({
  api: { deleteProfile: vi.fn(), deleteCredential: vi.fn(), listProfiles: vi.fn(), listCredentials: vi.fn() },
  errorMessage: (e: { message?: string }) => e?.message ?? String(e),
}));

function prof(name: string, group: string | null, host = `${name}.example.com`): Profile {
  return {
    name,
    protocol: 'sftp',
    host,
    port: null,
    user: null,
    auth: { method: 'agent' },
    remoteDir: null,
    group,
    credential: null,
  };
}

function renderSidebar() {
  const onconnectprofile = vi.fn();
  const oneditcredential = vi.fn();
  const onterminal = vi.fn();
  render(Sidebar, {
    props: {
      connecting: null,
      onconnectprofile,
      onquickconnect: vi.fn(),
      onnewprofile: vi.fn(),
      oneditprofile: vi.fn(),
      onnewcredential: vi.fn(),
      oneditcredential,
      ondisconnect: vi.fn(),
      onterminal,
    },
  });
  return { onconnectprofile, oneditcredential, onterminal };
}

const connectButtons = () =>
  screen.queryAllByRole('button', { name: /^Connect / }).map((b) => b.getAttribute('aria-label'));

beforeEach(() => {
  localStorage.clear();
  app.credentials = [];
  app.profiles = [prof('web', 'Work'), prof('db', 'Work', 'db.internal'), prof('nas', null), prof('shop', 'ACME')];
});

describe('Sidebar profiles', () => {
  it('lists ungrouped profiles first, then groups', () => {
    renderSidebar();
    expect(connectButtons()).toEqual(['Connect nas', 'Connect shop', 'Connect db', 'Connect web']);
    const work = screen.getByRole('button', { name: /Work/ });
    expect(work).toHaveAttribute('aria-expanded', 'true');
    expect(within(work).getByText('2')).toBeInTheDocument();
  });

  it('collapses a group and remembers it', async () => {
    const user = userEvent.setup();
    renderSidebar();
    await user.click(screen.getByRole('button', { name: /Work/ }));
    expect(connectButtons()).toEqual(['Connect nas', 'Connect shop']);
    expect(JSON.parse(localStorage.getItem('oxfer.collapsedGroups')!)).toEqual(['Work']);
  });

  it('search filters across groups, opens collapsed ones, Escape clears', async () => {
    const user = userEvent.setup();
    localStorage.setItem('oxfer.collapsedGroups', '["Work"]');
    const { onconnectprofile } = renderSidebar();
    expect(connectButtons()).not.toContain('Connect db');

    const search = screen.getByRole('searchbox', { name: 'Search profiles' });
    await user.type(search, 'internal');
    expect(connectButtons()).toEqual(['Connect db']);
    expect(screen.queryByRole('button', { name: /ACME/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Connect db' }));
    expect(onconnectprofile).toHaveBeenCalledWith(app.profiles[1]);

    await user.clear(search);
    await user.type(search, 'nothing');
    expect(screen.getByText('No profiles match.')).toBeInTheDocument();
    await user.type(search, '{Escape}');
    expect(search).toHaveValue('');
    expect(connectButtons()).toEqual(['Connect nas', 'Connect shop']);
  });

  it('lists credentials; deleting asks first and reports a credential in use', async () => {
    const user = userEvent.setup();
    const deploy = { name: 'deploy', user: 'ci', auth: { method: 'password' as const } };
    app.credentials = [deploy];
    vi.mocked(api.deleteCredential).mockRejectedValue({ code: 'inUse', message: "credential 'deploy' is used by web" });
    const { oneditcredential } = renderSidebar();
    const list = screen.getByRole('list', { name: 'Credentials' });
    expect(within(list).getByText('deploy')).toBeInTheDocument();
    expect(within(list).getByText('ci · Password')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Edit credential deploy' }));
    expect(oneditcredential).toHaveBeenCalledWith(deploy);
    await user.click(screen.getByRole('button', { name: 'Delete credential deploy' }));
    await user.click(screen.getByRole('button', { name: 'Delete' }));
    expect(api.deleteCredential).toHaveBeenCalledWith('deploy');
    expect(app.toasts.at(-1)?.message).toMatch(/used by web/);
  });
});

describe('Sidebar terminal', () => {
  it('offers a terminal for SFTP connections only', async () => {
    const user = userEvent.setup();
    app.sessions = [
      { id: 's1', label: 'me@box', protocol: 'sftp', home: '/' },
      { id: 's2', label: 'me@ftp', protocol: 'ftp', home: '/' },
    ];
    const { onterminal } = renderSidebar();
    expect(screen.queryByRole('button', { name: 'Open terminal me@ftp' })).toBeNull();
    await user.click(screen.getByRole('button', { name: 'Open terminal me@box' }));
    expect(onterminal).toHaveBeenCalledWith(app.sessions[0]);
    app.sessions = [];
  });
});
