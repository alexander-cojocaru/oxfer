<script lang="ts">
  import Icon from './Icon.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import { api } from '../lib/api';
  import { app } from '../lib/state.svelte';
  import { t } from '../lib/i18n';
  import type { Profile, SessionInfo } from '../lib/types';

  let {
    connecting,
    onconnectprofile,
    onquickconnect,
    onnewprofile,
    oneditprofile,
    ondisconnect,
  }: {
    connecting: string | null;
    onconnectprofile: (p: Profile) => void;
    onquickconnect: () => void;
    onnewprofile: () => void;
    oneditprofile: (p: Profile) => void;
    ondisconnect: (s: SessionInfo) => void;
  } = $props();

  let confirmDelete = $state<Profile | null>(null);
  const remotes = $derived(app.sessions.filter((s) => s.protocol !== 'local'));

  async function remove(p: Profile) {
    confirmDelete = null;
    try {
      await api.deleteProfile(p.name);
      await app.loadProfiles();
    } catch (e) {
      app.error(e);
    }
  }

  function protoLabel(p: Profile['protocol']) {
    return p === 'sftp' ? 'SFTP' : p === 'ftp' ? 'FTP' : 'FTPS';
  }
</script>

<aside aria-label={t('connections')}>
  <div class="actions">
    <button type="button" class="btn primary" onclick={onquickconnect} title="{t('quickConnect')} (Ctrl+N)">
      <Icon name="plug" />{t('quickConnect')}
    </button>
  </div>

  {#if remotes.length}
    <h3>{t('connections')}</h3>
    <ul>
      {#each remotes as s (s.id)}
        <li class="item">
          <span class="dot" aria-hidden="true"></span>
          <span class="label" title={s.label}>{s.label}</span>
          <button type="button" class="btn icon ghost" aria-label="{t('disconnect')} {s.label}" title={t('disconnect')} onclick={() => ondisconnect(s)}>
            <Icon name="x" size={14} />
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  <div class="head">
    <h3>{t('savedProfiles')}</h3>
    <button type="button" class="btn icon ghost" aria-label={t('newProfile')} title={t('newProfile')} onclick={onnewprofile}>
      <Icon name="plus" size={14} />
    </button>
  </div>
  {#if app.profiles.length === 0}
    <p class="empty">{t('noProfiles')}</p>
  {:else}
    <ul>
      {#each app.profiles as p (p.name)}
        <li class="item profile">
          <button
            type="button"
            class="connect"
            disabled={connecting !== null}
            aria-label="{t('connect')} {p.name}"
            onclick={() => onconnectprofile(p)}
          >
            <Icon name="server" size={14} />
            <span class="label">
              <span class="name">{p.name}</span>
              <span class="sub">{protoLabel(p.protocol)} · {p.user ? `${p.user}@` : ''}{p.host}</span>
            </span>
            {#if connecting === p.name}<span class="spinner" aria-label={t('connecting')}></span>{/if}
          </button>
          <button type="button" class="btn icon ghost" aria-label="{t('editProfile')} {p.name}" title={t('editProfile')} onclick={() => oneditprofile(p)}>
            <Icon name="edit" size={14} />
          </button>
          <button type="button" class="btn icon ghost" aria-label="{t('deleteProfile')} {p.name}" title={t('deleteProfile')} onclick={() => (confirmDelete = p)}>
            <Icon name="trash" size={14} />
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</aside>

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
    padding: 10px 8px;
    overflow: auto;
    min-height: 0;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .actions .btn {
    width: 100%;
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
