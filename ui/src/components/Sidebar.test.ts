import { render, screen, within } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Sidebar from './Sidebar.svelte';
import { api } from '../lib/api';
import { app } from '../lib/state.svelte';
import type { Profile } from '../lib/types';

vi.mock('../lib/api', () => ({
  api: { deleteProfile: vi.fn(), saveProfile: vi.fn(), deleteCredential: vi.fn(), listProfiles: vi.fn(), listCredentials: vi.fn() },
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
  const onprofileterminal = vi.fn();
  const oneditprofile = vi.fn();
  render(Sidebar, {
    props: {
      connecting: null,
      onconnectprofile,
      onnewprofile: vi.fn(),
      oneditprofile,
      onduplicateprofile: vi.fn(),
      onconfigtransfer: vi.fn(),
      onnewcredential: vi.fn(),
      oneditcredential,
      ondisconnect: vi.fn(),
      onterminal,
      onprofileterminal,
      onprofilescreen: vi.fn(),
    },
  });
  return { onconnectprofile, oneditcredential, onterminal, onprofileterminal, oneditprofile };
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

describe('Sidebar sections', () => {
  it('collapses profiles and credentials independently and remembers it', async () => {
    const user = userEvent.setup();
    app.credentials = [{ name: 'deploy', user: 'ci', auth: { method: 'password' } }];
    renderSidebar();
    const profiles = screen.getByRole('button', { name: /^Saved profiles/ });
    const credentials = screen.getByRole('button', { name: /^Credentials/ });
    expect(profiles).toHaveAttribute('aria-expanded', 'true');

    await user.click(profiles);
    expect(profiles).toHaveAttribute('aria-expanded', 'false');
    expect(connectButtons()).toEqual([]);
    expect(screen.queryByRole('searchbox')).toBeNull();
    expect(screen.getByRole('list', { name: 'Credentials' })).toBeInTheDocument();

    await user.click(credentials);
    expect(screen.queryByRole('list', { name: 'Credentials' })).toBeNull();
    expect(JSON.parse(localStorage.getItem('oxfer.collapsedSections')!)).toEqual(['profiles', 'credentials']);

    await user.click(profiles);
    expect(connectButtons()).toContain('Connect nas');
  });
});

describe('Sidebar profile menu', () => {
  const rightClick = (name: string) =>
    userEvent.setup().pointer({ keys: '[MouseRight]', target: screen.getByRole('button', { name: `Connect ${name}` }) });
  const menuItems = () => screen.getAllByRole('menuitem').map((b) => b.textContent?.trim());

  it('keeps the row free of action icons; left click connects', async () => {
    const { onconnectprofile } = renderSidebar();
    expect(screen.queryByRole('button', { name: /^(Edit|Delete) profile / })).toBeNull();
    await userEvent.setup().click(screen.getByRole('button', { name: 'Connect nas' }));
    expect(onconnectprofile).toHaveBeenCalledWith(app.profiles[2]);
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('right click offers terminal, move, edit and delete', async () => {
    const user = userEvent.setup();
    app.profiles = [...app.profiles, { ...prof('legacy', null), protocol: 'ftp' }];
    const { onprofileterminal, oneditprofile } = renderSidebar();

    await rightClick('legacy');
    expect(menuItems()).not.toContain('Open terminal');
    await user.keyboard('{Escape}');

    await rightClick('web');
    expect(menuItems()).toEqual(['Connect', 'Open terminal', 'Open screen (VNC)', 'Move to group…', 'Edit profile…', 'Duplicate profile…', 'Delete profile…']);
    await user.click(screen.getByRole('menuitem', { name: 'Open terminal' }));
    expect(onprofileterminal).toHaveBeenCalledWith(app.profiles[0]);
    expect(screen.queryByRole('menu')).toBeNull();

    await rightClick('web');
    await user.click(screen.getByRole('menuitem', { name: 'Edit profile…' }));
    expect(oneditprofile).toHaveBeenCalledWith(app.profiles[0]);

    vi.mocked(api.saveProfile).mockResolvedValue(undefined as never);
    vi.mocked(api.listProfiles).mockResolvedValue(app.profiles as never);
    vi.mocked(api.listCredentials).mockResolvedValue([] as never);
    const web = app.profiles[0];
    await rightClick('web');
    await user.click(screen.getByRole('menuitem', { name: 'Move to group…' }));
    const select = screen.getByRole('combobox', { name: 'Move to group web' });
    expect(select).toHaveFocus();
    await user.selectOptions(select, 'ACME');
    expect(api.saveProfile).toHaveBeenCalledWith('web', { ...web, group: 'ACME' }, null);

    await rightClick('nas');
    await user.click(screen.getByRole('menuitem', { name: 'Delete profile…' }));
    await user.click(screen.getByRole('button', { name: 'Delete' }));
    expect(api.deleteProfile).toHaveBeenCalledWith('nas');
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
