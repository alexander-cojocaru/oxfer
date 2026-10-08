<script lang="ts">
  import Modal from './Modal.svelte';
  import { api, errorMessage } from '../lib/api';
  import { t } from '../lib/i18n';
  import type { AuthDto, Credential } from '../lib/types';

  let {
    credential = null,
    onsaved,
    onclose,
  }: {
    /** `null` creates a new credential. */
    credential?: Credential | null;
    onsaved?: (c: Credential) => void;
    onclose: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  const initial = credential;
  let name = $state(initial?.name ?? '');
  let user = $state(initial?.user ?? '');
  let auth = $state<AuthDto['method']>(initial?.auth.method ?? 'password');
  let keyPath = $state(initial?.auth.method === 'keyFile' ? initial.auth.path : '');
  let password = $state('');
  let busy = $state(false);
  let error = $state<string | null>(null);

  const valid = $derived(name.trim() !== '' && (auth !== 'keyFile' || keyPath.trim() !== ''));

  function buildAuth(): AuthDto {
    return auth === 'keyFile' ? { method: 'keyFile', path: keyPath.trim() } : { method: auth };
  }

  async function submit(e: Event) {
    e.preventDefault();
    if (!valid || busy) return;
    busy = true;
    error = null;
    try {
      const saved = await api.saveCredential(
        initial?.name ?? null,
        { name: name.trim(), user: user.trim() || null, auth: buildAuth() },
        auth === 'password' && password ? password : null,
      );
      onsaved?.(saved);
      onclose();
    } catch (err) {
      error = errorMessage(err);
    } finally {
      busy = false;
      password = '';
    }
  }
</script>

<Modal title={initial ? t('editCredential') : t('newCredential')} onclose={busy ? undefined : onclose} width={460}>
  <form id="credential-form" class="grid" onsubmit={submit} aria-busy={busy}>
    <label class="field wide">
      <span>{t('credentialName')}</span>
      <input bind:value={name} required data-autofocus autocomplete="off" placeholder={t('credentialNamePlaceholder')} />
    </label>
    <label class="field">
      <span>{t('user')}</span>
      <input bind:value={user} spellcheck="false" autocomplete="off" />
    </label>
    <label class="field">
      <span>{t('authentication')}</span>
      <select bind:value={auth}>
        <option value="password">{t('authPassword')}</option>
        <option value="keyFile">{t('authKeyFile')}</option>
        <option value="agent">{t('authAgent')}</option>
        <option value="interactive">{t('authInteractive')}</option>
      </select>
    </label>
    {#if auth === 'keyFile'}
      <label class="field wide">
        <span>{t('keyFile')}</span>
        <input bind:value={keyPath} class="mono" spellcheck="false" placeholder="~/.ssh/id_ed25519" />
      </label>
    {/if}
    {#if auth === 'password'}
      <div class="field wide">
        <label class="field">
          <span>{t('password')}</span>
          <input type="password" bind:value={password} autocomplete="off" aria-describedby="credential-password-hint" />
        </label>
        <small id="credential-password-hint" class="hint">{t('passwordStoredHint')}</small>
      </div>
    {/if}
    <p class="hint wide">{t('credentialHint')}</p>
    {#if error}
      <p class="error wide" role="alert">{error}</p>
    {/if}
  </form>
  {#snippet footer()}
    <button type="button" class="btn" onclick={onclose} disabled={busy}>{t('cancel')}</button>
    <button type="submit" form="credential-form" class="btn primary" disabled={!valid || busy}>{t('save')}</button>
  {/snippet}
</Modal>

<style>
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .wide {
    grid-column: 1 / -1;
  }
  p.hint {
    margin: 0;
  }
  .error {
    margin: 0;
    padding: 8px 10px;
    border-radius: var(--radius-sm);
    background: var(--danger-soft);
    color: var(--danger);
    overflow-wrap: anywhere;
  }
  @media (max-width: 520px) {
    .grid {
      grid-template-columns: 1fr;
    }
  }
</style>
