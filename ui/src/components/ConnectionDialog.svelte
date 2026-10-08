<script lang="ts">
  import Modal from './Modal.svelte';
  import { api, errorMessage } from '../lib/api';
  import { t } from '../lib/i18n';
  import type { AuthDto, Profile, QuickAuth, RemoteProtocol, SessionInfo } from '../lib/types';

  type AuthChoice = QuickAuth | 'interactive';

  let {
    mode,
    profile = null,
    onconnected,
    onsaved,
    onclose,
  }: {
    /** `quick`: connect now (optionally saving); `profile`: create/edit a saved profile. */
    mode: 'quick' | 'profile';
    profile?: Profile | null;
    onconnected?: (s: SessionInfo) => void;
    onsaved?: (p: Profile) => void;
    onclose: () => void;
  } = $props();

  const DEFAULT_PORTS: Record<RemoteProtocol, number> = { sftp: 22, ftp: 21, ftpsExplicit: 21, ftpsImplicit: 990 };

  // svelte-ignore state_referenced_locally
  const initial = profile;
  let name = $state(initial?.name ?? '');
  let protocol = $state<RemoteProtocol>(initial?.protocol ?? 'sftp');
  let host = $state(initial?.host ?? '');
  let port = $state<string>(initial?.port ? String(initial.port) : '');
  let user = $state(initial?.user ?? '');
  // svelte-ignore state_referenced_locally
  let auth = $state<AuthChoice>(
    initial ? (initial.auth.method as AuthChoice) : mode === 'quick' ? 'auto' : 'agent',
  );
  let keyPath = $state(initial?.auth.method === 'keyFile' ? initial.auth.path : '');
  let password = $state('');
  let remoteDir = $state(initial?.remoteDir ?? '');
  let insecureTls = $state(false);
  let saveAsProfile = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);

  const isSftp = $derived(protocol === 'sftp');
  const saving = $derived(mode === 'profile' || saveAsProfile);
  const authChoices = $derived.by((): { value: AuthChoice; label: string }[] => {
    if (isSftp) {
      const base: { value: AuthChoice; label: string }[] = [
        { value: 'agent', label: t('authAgent') },
        { value: 'keyFile', label: t('authKeyFile') },
        { value: 'password', label: t('authPassword') },
      ];
      return mode === 'quick' && !saveAsProfile
        ? [{ value: 'auto', label: t('authAuto') }, ...base]
        : [...base, { value: 'interactive', label: t('authInteractive') }];
    }
    return mode === 'quick' && !saveAsProfile
      ? [{ value: 'password', label: t('authPassword') }]
      : [
          { value: 'password', label: t('authPassword') },
          { value: 'interactive', label: t('authInteractive') },
        ];
  });

  $effect(() => {
    // keep the auth choice valid for the protocol/mode
    if (!authChoices.some((c) => c.value === auth)) auth = authChoices[0].value;
  });

  const valid = $derived(
    host.trim() !== '' &&
      (!saving || name.trim() !== '') &&
      (port === '' || (/^\d+$/.test(port) && +port > 0 && +port < 65536)) &&
      (auth !== 'keyFile' || keyPath.trim() !== ''),
  );

  function profileAuth(): AuthDto {
    switch (auth) {
      case 'keyFile':
        return { method: 'keyFile', path: keyPath.trim() };
      case 'password':
        return { method: 'password' };
      case 'interactive':
        return { method: 'interactive' };
      default:
        return { method: 'agent' };
    }
  }

  function buildProfile(): Profile {
    return {
      name: name.trim(),
      protocol,
      host: host.trim(),
      port: port ? Number(port) : null,
      user: user.trim() || null,
      auth: profileAuth(),
      remoteDir: remoteDir.trim() || null,
    };
  }

  async function submit(e: Event) {
    e.preventDefault();
    if (!valid || busy) return;
    busy = true;
    error = null;
    try {
      if (saving) {
        const saved = await api.saveProfile(
          mode === 'profile' ? (initial?.name ?? null) : null,
          buildProfile(),
          auth === 'password' && password ? password : null,
        );
        onsaved?.(saved);
        if (mode === 'profile') {
          onclose();
          return;
        }
        const s = await api.connect({ kind: 'profile', name: saved.name });
        onconnected?.(s);
      } else {
        const s = await api.connect({
          kind: 'quick',
          protocol,
          host: host.trim(),
          port: port ? Number(port) : null,
          user: user.trim() || null,
          auth: auth as QuickAuth,
          password: password || null,
          keyPath: auth === 'keyFile' ? keyPath.trim() : null,
          remoteDir: remoteDir.trim() || null,
          insecureTls,
        });
        onconnected?.(s);
      }
      onclose();
    } catch (err) {
      error = errorMessage(err);
    } finally {
      busy = false;
      password = '';
    }
  }

  const title = $derived(mode === 'quick' ? t('quickConnect') : initial ? t('editProfile') : t('newProfile'));
</script>

<Modal {title} onclose={busy ? undefined : onclose} width={520}>
  <form id="connection-form" class="grid" onsubmit={submit} aria-busy={busy}>
    {#if saving}
      <label class="field wide">
        <span>{t('profileName')}</span>
        <input bind:value={name} required data-autofocus={mode === 'profile' ? true : undefined} autocomplete="off" />
      </label>
    {/if}
    <label class="field">
      <span>{t('protocol')}</span>
      <select bind:value={protocol}>
        <option value="sftp">SFTP</option>
        <option value="ftpsExplicit">FTPS (explicit TLS)</option>
        <option value="ftpsImplicit">FTPS (implicit TLS)</option>
        <option value="ftp">FTP (unencrypted)</option>
      </select>
    </label>
    <label class="field">
      <span>{t('port')}</span>
      <input
        bind:value={port}
        inputmode="numeric"
        placeholder={String(DEFAULT_PORTS[protocol])}
        aria-invalid={port !== '' && !/^\d+$/.test(port)}
      />
    </label>
    <label class="field wide">
      <span>{t('host')}</span>
      <input
        bind:value={host}
        required
        spellcheck="false"
        autocomplete="off"
        placeholder="example.com"
        data-autofocus={mode === 'quick' ? true : undefined}
      />
    </label>
    <label class="field">
      <span>{t('user')}</span>
      <input bind:value={user} spellcheck="false" autocomplete="off" placeholder={isSftp ? '' : 'anonymous'} />
    </label>
    <label class="field">
      <span>{t('authentication')}</span>
      <select bind:value={auth}>
        {#each authChoices as c (c.value)}
          <option value={c.value}>{c.label}</option>
        {/each}
      </select>
    </label>
    {#if auth === 'keyFile'}
      <label class="field wide">
        <span>{t('keyFile')}</span>
        <input bind:value={keyPath} class="mono" spellcheck="false" placeholder="~/.ssh/id_ed25519" />
      </label>
    {/if}
    {#if auth === 'password' || (auth === 'auto' && !saving)}
      <div class="field wide">
        <label class="field">
          <span>{t('password')}</span>
          <input type="password" bind:value={password} autocomplete="off" aria-describedby="password-hint" />
        </label>
        <small id="password-hint" class="hint">{saving ? t('passwordStoredHint') : t('passwordOnceHint')}</small>
      </div>
    {/if}
    <label class="field wide">
      <span>{t('remoteDir')}</span>
      <input bind:value={remoteDir} class="mono" spellcheck="false" placeholder="~" />
    </label>
    {#if protocol === 'ftp'}
      <p class="warn wide" role="note">{t('plainFtpWarning')}</p>
    {/if}
    {#if mode === 'quick' && (protocol === 'ftpsExplicit' || protocol === 'ftpsImplicit')}
      <label class="check wide">
        <input type="checkbox" bind:checked={insecureTls} />
        {t('insecureTls')}
      </label>
    {/if}
    {#if mode === 'quick'}
      <label class="check wide">
        <input type="checkbox" bind:checked={saveAsProfile} />
        {t('saveAsProfile')}
      </label>
    {/if}
    {#if error}
      <p class="error wide" role="alert">{error}</p>
    {/if}
  </form>
  {#snippet footer()}
    <button type="button" class="btn" onclick={onclose} disabled={busy}>{t('cancel')}</button>
    <button type="submit" form="connection-form" class="btn primary" disabled={!valid || busy}>
      {busy ? t('connecting') : mode === 'profile' ? t('save') : t('connect')}
    </button>
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
  .check {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .check input {
    min-height: auto;
  }
  .warn {
    margin: 0;
    padding: 8px 10px;
    border-radius: var(--radius-sm);
    background: var(--warn-soft);
    color: var(--warn);
  }
  .error {
    margin: 0;
    padding: 8px 10px;
    border-radius: var(--radius-sm);
    background: var(--danger-soft);
    color: var(--danger);
    overflow-wrap: anywhere;
  }
  @media (max-width: 560px) {
    .grid {
      grid-template-columns: 1fr;
    }
  }
</style>
