<script lang="ts">
  import Icon from './Icon.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import ContextMenu, { type MenuEntry } from './ContextMenu.svelte';
  import { api } from '../lib/api';
  import { app } from '../lib/state.svelte';
  import { t } from '../lib/i18n';
  import { groupNames, groupProfiles, parentGroups } from '../lib/profiles';
  import type { Credential, Profile, SessionInfo } from '../lib/types';

  let {
    connecting,
    onconnectprofile,
    onnewprofile,
    oneditprofile,
    onduplicateprofile,
    onconfigtransfer,
    onnewcredential,
    oneditcredential,
    ondisconnect,
    onterminal,
    onprofileterminal,
    onprofilescreen,
  }: {
    connecting: string | null;
    onconnectprofile: (p: Profile) => void;
    onnewprofile: () => void;
    oneditprofile: (p: Profile) => void;
    onduplicateprofile: (p: Profile) => void;
    onconfigtransfer: (mode: 'export' | 'import') => void;
    onnewcredential: () => void;
    oneditcredential: (c: Credential) => void;
    ondisconnect: (s: SessionInfo) => void;
    onterminal: (s: SessionInfo) => void;
    onprofileterminal: (p: Profile) => void;
    /** Opens the screen (VNC) of a file profile's host. */
    onprofilescreen: (p: Profile) => void;
  } = $props();

  let confirmDelete = $state<Profile | null>(null);
  let confirmDeleteCredential = $state<Credential | null>(null);

  async function removeCredential(c: Credential) {
    confirmDeleteCredential = null;
    try {
      await api.deleteCredential(c.name);
      await app.loadProfiles();
    } catch (e) {
      app.error(e);
    }
  }

  function authLabel(a: Credential['auth']) {
    switch (a.method) {
      case 'agent':
        return t('authAgent');
      case 'keyFile':
        return t('authKeyFile');
      case 'password':
        return t('authPassword');
      default:
        return t('authInteractive');
    }
  }
  const remotes = $derived(app.sessions.filter((s) => s.protocol !== 'local'));

  let query = $state('');
  const groups = $derived(groupProfiles(app.profiles, query));

  // Collapsed groups survive restarts; storage may be unavailable.
  const COLLAPSED_KEY = 'oxfer.collapsedGroups';
  function loadCollapsed(): Set<string> {
    try {
      const v: unknown = JSON.parse(localStorage.getItem(COLLAPSED_KEY) ?? '[]');
      return new Set(Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []);
    } catch {
      return new Set();
    }
  }
  let collapsed = $state(loadCollapsed());

  function toggleGroup(name: string) {
    const next = new Set(collapsed);
    if (!next.delete(name)) next.add(name);
    collapsed = next;
    try {
      localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...next]));
    } catch {
      // not persisted; fine
    }
  }

  // Collapsed sidebar sections survive restarts too.
  const SECTIONS_KEY = 'oxfer.collapsedSections';
  type Section = 'profiles' | 'credentials';
  function loadSections(): Set<Section> {
    try {
      const v: unknown = JSON.parse(localStorage.getItem(SECTIONS_KEY) ?? '[]');
      return new Set(Array.isArray(v) ? v.filter((x): x is Section => x === 'profiles' || x === 'credentials') : []);
    } catch {
      return new Set();
    }
  }
  let closedSections = $state(loadSections());
  const profilesOpen = $derived(!closedSections.has('profiles'));
  const credentialsOpen = $derived(!closedSections.has('credentials'));

  function toggleSection(name: Section) {
    const next = new Set(closedSections);
    if (!next.delete(name)) next.add(name);
    closedSections = next;
    try {
      localStorage.setItem(SECTIONS_KEY, JSON.stringify([...next]));
    } catch {
      // not persisted; fine
    }
  }

  // While searching, show every match regardless of collapsed groups.
  const isOpen = (name: string | null) => name === null || query.trim() !== '' || !collapsed.has(name);
  // Depth is unlimited; indentation stops growing so deep trees keep room for the labels.
  const indent = (depth: number) => Math.min(depth, 8) * 14;
  // A sub group is hidden while any of its parents is collapsed.
  const isShown = (name: string | null) => name === null || parentGroups(name).every(isOpen);

  async function remove(p: Profile) {
    confirmDelete = null;
    try {
      await api.deleteProfile(p.name);
      await app.loadProfiles();
    } catch (e) {
      app.error(e);
    }
  }

  // Moving only changes the group; the name stays, so a stored password is kept.
  let moving = $state<string | null>(null);
  let dragged = $state<string | null>(null);
  let dropTarget = $state<string | null>(null);
  const allGroups = $derived(groupNames(app.profiles));

  const focusOnMount = (el: HTMLElement) => el.focus();

  async function moveTo(name: string, group: string | null) {
    moving = null;
    dragged = null;
    dropTarget = null;
    const p = app.profiles.find((x) => x.name === name);
    if (!p || (p.group ?? null) === group) return;
    try {
      await api.saveProfile(p.name, { ...p, group }, null);
      await app.loadProfiles();
    } catch (e) {
      app.error(e);
    }
  }

  // `target` is a group path, or `''` for the ungrouped list.
  const dropProps = (target: string) => ({
    ondragover: (e: DragEvent) => {
      if (dragged === null) return;
      e.preventDefault();
      dropTarget = target;
    },
    ondragleave: () => {
      if (dropTarget === target) dropTarget = null;
    },
    ondrop: (e: DragEvent) => {
      e.preventDefault();
      if (dragged !== null) void moveTo(dragged, target || null);
    },
  });

  function protoLabel(p: Profile['protocol']) {
    return p === 'sftp' ? 'SFTP' : p === 'ftp' ? 'FTP' : p === 'vnc' ? 'VNC' : 'FTPS';
  }

  // Left click connects; everything else lives in the right-click menu.
  let menu = $state<{ x: number; y: number; items: MenuEntry[] } | null>(null);

  function openProfileMenu(e: MouseEvent, p: Profile) {
    e.preventDefault();
    // Opened from the keyboard (menu key, Shift+F10) there is no pointer position.
    let { clientX: x, clientY: y } = e;
    if (x === 0 && y === 0) {
      const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
      x = r.left + 24;
      y = r.bottom;
    }
    const busy = connecting !== null;
    const items: MenuEntry[] = [{ label: t('connect'), action: () => onconnectprofile(p), disabled: busy }];
    if (p.protocol === 'sftp') {
      items.push({ label: t('openTerminal'), action: () => onprofileterminal(p), disabled: busy });
    }
    if (p.protocol !== 'vnc') {
      items.push({ label: t('openScreen'), action: () => onprofilescreen(p), disabled: busy });
    }
    items.push(
      'separator',
      { label: `${t('moveProfile')}…`, action: () => (moving = p.name) },
      { label: `${t('editProfile')}…`, action: () => oneditprofile(p) },
      { label: `${t('duplicateProfile')}…`, action: () => onduplicateprofile(p) },
      'separator',
      { label: `${t('deleteProfile')}…`, action: () => (confirmDelete = p), danger: true },
    );
    menu = { x, y, items };
  }

  // Tooltip for the (possibly ellipsized) profile row: full name plus target.
  function profileTitle(p: Profile) {
    const user = p.user && !p.credential ? `${p.user}@` : '';
    return `${p.name}\n${protoLabel(p.protocol)} · ${user}${p.host}\n${t('profileMenuHint')}`;
  }
</script>

<aside aria-label={t('connections')}>
  {#if remotes.length}
    <h3>{t('connections')}</h3>
    <ul class="sessions">
      {#each remotes as s (s.id)}
        <li class="item">
          <span class="dot" aria-hidden="true"></span>
          <span class="label" title={s.label}>{s.label}</span>
          {#if s.protocol === 'sftp'}
            <button type="button" class="btn icon ghost" aria-label="{t('openTerminal')} {s.label}" title={t('openTerminal')} onclick={() => onterminal(s)}>
              <Icon name="terminal" size={14} />
            </button>
          {/if}
          <button type="button" class="btn icon ghost" aria-label="{t('disconnect')} {s.label}" title={t('disconnect')} onclick={() => ondisconnect(s)}>
            <Icon name="x" size={14} />
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  <section class="section profiles" class:closed={!profilesOpen}>
  <div class="head" class:drop={dropTarget === ''} {...dropProps('')}>
    <h3>
      <button type="button" class="toggle" aria-expanded={profilesOpen} onclick={() => toggleSection('profiles')}>
        <span class="chevron" class:open={profilesOpen} aria-hidden="true">▸</span>{t('savedProfiles')}
        <span class="count">{app.profiles.length}</span>
      </button>
    </h3>
    <button
      type="button"
      class="btn icon ghost"
      aria-label={t('moreActions')}
      title={t('moreActions')}
      aria-haspopup="menu"
      onclick={(e) => {
        const r = e.currentTarget.getBoundingClientRect();
        menu = {
          x: r.left,
          y: r.bottom,
          items: [
            { label: `${t('importProfiles')}…`, action: () => onconfigtransfer('import') },
            { label: `${t('exportProfiles')}…`, action: () => onconfigtransfer('export') },
          ],
        };
      }}>⋯</button
    >
    <button type="button" class="btn icon ghost" aria-label={t('newProfile')} title={t('newProfile')} onclick={onnewprofile}>
      <Icon name="plus" size={14} />
    </button>
  </div>
  {#if !profilesOpen}
    <!-- collapsed: header only -->
  {:else if app.profiles.length === 0}
    <p class="empty">{t('noProfiles')}</p>
  {:else}
    <div class="search">
      <Icon name="search" size={14} />
      <input
        bind:value={query}
        type="search"
        placeholder={t('searchProfiles')}
        aria-label={t('searchProfiles')}
        spellcheck="false"
        autocomplete="off"
        onkeydown={(e) => {
          if (e.key === 'Escape' && query) {
            e.stopPropagation();
            query = '';
          }
        }}
      />
    </div>
    <div class="scroll">
    {#if groups.length === 0}
      <p class="empty">{t('noProfileMatches')}</p>
    {/if}
    {#each groups as g (g.name ?? '')}
      {#if isShown(g.name)}
      {#if g.name !== null}
        {@const open = isOpen(g.name)}
        <button
          type="button"
          class="group"
          class:drop={dropTarget === g.name}
          {...dropProps(g.name)}
          style:padding-left="{6 + indent(g.depth)}px"
          aria-expanded={open}
          onclick={() => toggleGroup(g.name!)}
        >
          <span class="chevron" class:open aria-hidden="true">▸</span>
          <Icon name="folder" size={14} />
          <span class="group-name" title={g.name}>{g.label}</span>
          <span class="count">{g.total}</span>
        </button>
      {/if}
      {#if isOpen(g.name)}
        <ul
          class:nested={g.name !== null}
          style:padding-left={g.name !== null ? `${14 + indent(g.depth)}px` : undefined}
          aria-label={g.name ?? t('savedProfiles')}
        >
          {#each g.profiles as p (p.name)}
            <li
              class="item profile"
              class:dragging={dragged === p.name}
              draggable="true"
              ondragstart={(e) => {
                dragged = p.name;
                e.dataTransfer?.setData('text/plain', p.name);
                if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
              }}
              ondragend={() => {
                dragged = null;
                dropTarget = null;
              }}
              oncontextmenu={(e) => openProfileMenu(e, p)}
            >
              <button
                type="button"
                class="connect"
                disabled={connecting !== null}
                aria-label="{t('connect')} {p.name}"
                title={profileTitle(p)}
                onclick={() => onconnectprofile(p)}
              >
                <Icon name={p.protocol === 'vnc' ? 'monitor' : 'server'} size={14} />
                <span class="label">
                  <span class="name">{p.name}</span>
                  <span class="sub">
                    {protoLabel(p.protocol)} · {p.user && !p.credential ? `${p.user}@` : ''}{p.host}{#if p.credential}
                      · <Icon name="key" size={10} /> {p.credential}{/if}
                  </span>
                </span>
                {#if connecting === p.name}<span class="spinner" aria-label={t('connecting')}></span>{/if}
              </button>
              {#if p.protocol !== 'vnc'}
                <button
                  type="button"
                  class="btn icon ghost screen"
                  disabled={connecting !== null}
                  aria-label="{t('openScreen')} {p.name}"
                  title={t('openScreen')}
                  onclick={() => onprofilescreen(p)}
                >
                  <Icon name="monitor" size={14} />
                </button>
              {/if}
            </li>
            {#if moving === p.name}
              <li class="move">
                <select
                  aria-label="{t('moveProfile')} {p.name}"
                  value={p.group ?? ''}
                  use:focusOnMount
                  onblur={() => (moving = null)}
                  onchange={(e) => moveTo(p.name, e.currentTarget.value || null)}
                  onkeydown={(e) => {
                    if (e.key === 'Escape') {
                      e.stopPropagation();
                      moving = null;
                    }
                  }}
                >
                  <option value="">{t('noGroup')}</option>
                  {#each allGroups as name (name)}
                    <option value={name}>{name}</option>
                  {/each}
                </select>
              </li>
            {/if}
          {/each}
        </ul>
      {/if}
      {/if}
    {/each}
    </div>
  {/if}
  </section>

  <section class="section credentials" class:closed={!credentialsOpen}>
  <div class="head">
    <h3>
      <button type="button" class="toggle" aria-expanded={credentialsOpen} onclick={() => toggleSection('credentials')}>
        <span class="chevron" class:open={credentialsOpen} aria-hidden="true">▸</span>{t('credentials')}
        <span class="count">{app.credentials.length}</span>
      </button>
    </h3>
    <button type="button" class="btn icon ghost" aria-label={t('newCredential')} title={t('newCredential')} onclick={onnewcredential}>
      <Icon name="plus" size={14} />
    </button>
  </div>
  {#if !credentialsOpen}
    <!-- collapsed: header only -->
  {:else if app.credentials.length === 0}
    <p class="empty">{t('noCredentials')}</p>
  {:else}
    <ul class="scroll" aria-label={t('credentials')}>
      {#each app.credentials as c (c.name)}
        <li class="item">
          <Icon name="key" size={14} />
          <span class="label" title={c.name}>
            <span class="name">{c.name}</span>
            <span class="sub">{c.user ? `${c.user} · ` : ''}{authLabel(c.auth)}</span>
          </span>
          <button type="button" class="btn icon ghost" aria-label="{t('editCredential')} {c.name}" title={t('editCredential')} onclick={() => oneditcredential(c)}>
            <Icon name="edit" size={14} />
          </button>
          <button type="button" class="btn icon ghost" aria-label="{t('deleteCredential')} {c.name}" title={t('deleteCredential')} onclick={() => (confirmDeleteCredential = c)}>
            <Icon name="trash" size={14} />
          </button>
        </li>
      {/each}
    </ul>
  {/if}
  </section>
</aside>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />
{/if}

{#if confirmDeleteCredential}
  {@const c = confirmDeleteCredential}
  <ConfirmDialog
    title={t('deleteCredential')}
    message={t('deleteCredentialConfirm', { name: c.name })}
    confirmLabel={t('delete')}
    danger
    onconfirm={() => removeCredential(c)}
    oncancel={() => (confirmDeleteCredential = null)}
  />
{/if}

{#if confirmDelete}
  {@const p = confirmDelete}
  <ConfirmDialog
    title={t('deleteProfile')}
    message={t('deleteProfileConfirm', { name: p.name })}
    confirmLabel={t('delete')}
    danger
    onconfirm={() => remove(p)}
    oncancel={() => (confirmDelete = null)}
  />
{/if}

<style>
  aside {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 2px 8px 10px;
    overflow: hidden;
    min-height: 0;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .sessions {
    flex: none;
    max-height: 30%;
    overflow: auto;
  }
  /* Profiles and credentials each get a fixed share of the sidebar and scroll
     on their own; a collapsed section shrinks to its header. */
  .section {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .section.profiles {
    flex: 3 1 0;
  }
  .section.credentials {
    flex: 2 1 0;
  }
  .section.closed {
    flex: none;
  }
  .scroll {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    border-top: 1px solid var(--border);
    padding-top: 4px;
  }
  .toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 4px;
    margin: 0 -4px;
    background: none;
    border: none;
    border-radius: var(--radius-sm);
    font: inherit;
    letter-spacing: inherit;
    text-transform: inherit;
    color: inherit;
  }
  .toggle:hover {
    color: var(--fg-muted);
    background: var(--bg-hover);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h3 {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--fg-faint);
    margin: 12px 4px 4px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .item {
    color: var(--fg-muted);
    display: flex;
    align-items: center;
    gap: 4px;
    border-radius: var(--radius-sm);
    padding: 2px 2px 2px 6px;
  }
  .item:hover {
    background: var(--bg-hover);
  }
  .item .btn {
    opacity: 0.6;
  }
  .item:hover .btn,
  .item .btn:focus-visible {
    opacity: 1;
  }
  .profile {
    padding-left: 0;
  }
  .connect {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 6px;
    background: none;
    border: none;
    text-align: left;
    color: var(--fg-muted);
    border-radius: var(--radius-sm);
  }
  .label {
    min-width: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    flex: 1;
  }
  .name {
    color: var(--fg);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--ok);
    flex: none;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 2px 4px;
    padding-left: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg);
    color: var(--fg-faint);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--fg);
    outline: none;
  }
  .search:focus-within {
    outline: 2px solid var(--focus);
    outline-offset: 1px;
  }
  .group {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 4px 6px;
    margin-top: 4px;
    background: none;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--fg-muted);
    font-weight: 500;
    text-align: left;
  }
  .group:hover {
    background: var(--bg-hover);
  }
  .group-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chevron {
    width: 10px;
    font-size: 10px;
    transition: transform 0.12s;
  }
  .chevron.open {
    transform: rotate(90deg);
  }
  .count {
    font-size: 11px;
    color: var(--fg-faint);
  }
  .drop {
    outline: 2px dashed var(--accent);
    outline-offset: -2px;
  }
  .dragging {
    opacity: 0.5;
  }
  .move {
    list-style: none;
    padding: 0 4px 4px 22px;
  }
  .move select {
    width: 100%;
  }
  ul.nested {
    padding-left: 14px;
  }
  .empty {
    color: var(--fg-faint);
    margin: 4px;
    font-size: 12px;
  }
  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
