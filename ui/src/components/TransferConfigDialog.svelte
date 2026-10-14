<script lang="ts">
  import Modal from './Modal.svelte';
  import { api, errorMessage } from '../lib/api';
  import { app } from '../lib/state.svelte';
  import { t } from '../lib/i18n';

  let { mode, onclose }: { mode: 'export' | 'import'; onclose: () => void } = $props();

  let path = $state('');
  let overwrite = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);

  async function submit(e: Event) {
    e.preventDefault();
    if (!path.trim() || busy) return;
    busy = true;
    error = null;
    try {
      if (mode === 'export') {
        const n = await api.exportProfiles(path.trim());
        app.toast('success', t('exported', { count: n }));
      } else {
        const r = await api.importProfiles(path.trim(), overwrite);
        await app.loadProfiles();
        app.toast('success', t('imported', { added: r.added, replaced: r.replaced, skipped: r.skipped }));
      }
      onclose();
    } catch (err) {
      error = errorMessage(err);
    } finally {
      busy = false;
    }
  }
</script>

<Modal title={mode === 'export' ? t('exportProfiles') : t('importProfiles')} onclose={busy ? undefined : onclose}>
  <form id="transfer-config" onsubmit={submit}>
    <label class="field">
      <span>{t('configFile')}</span>
      <input bind:value={path} data-autofocus spellcheck="false" autocomplete="off" placeholder="~/oxfer-profiles.toml" />
    </label>
    {#if mode === 'import'}
      <label class="check">
        <input type="checkbox" bind:checked={overwrite} />
        <span>{t('overwriteExisting')}</span>
      </label>
    {/if}
    <small class="hint">{t('noPasswordsHint')}</small>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  </form>
  {#snippet footer()}
    <button type="button" class="btn" onclick={onclose} disabled={busy}>{t('cancel')}</button>
    <button type="submit" form="transfer-config" class="btn primary" disabled={!path.trim() || busy}>
      {mode === 'export' ? t('export') : t('import')}
    </button>
  {/snippet}
</Modal>

<style>
  .check {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-top: 10px;
  }
  .hint {
    display: block;
    margin-top: 10px;
    color: var(--text-dim, inherit);
    opacity: 0.8;
  }
  .error {
    color: var(--danger);
    font-size: 12px;
    margin: 8px 0 0;
  }
</style>
