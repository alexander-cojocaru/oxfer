<script lang="ts">
  import { onMount } from 'svelte';
  import Modal from './Modal.svelte';
  import { api, errorMessage } from '../lib/api';
  import { t } from '../lib/i18n';
  import type { UpdateInfo } from '../lib/types';

  let { activeTransfers = 0, onclose }: { activeTransfers?: number; onclose: () => void } = $props();

  type Phase =
    | { kind: 'checking' }
    | { kind: 'current' }
    | { kind: 'available'; update: UpdateInfo }
    | { kind: 'downloading'; received: number; total: number | null }
    | { kind: 'installing' }
    | { kind: 'ready' }
    | { kind: 'error'; message: string; retry: () => void };

  let phase = $state<Phase>({ kind: 'checking' });
  const busy = $derived(phase.kind === 'downloading' || phase.kind === 'installing');

  async function check() {
    phase = { kind: 'checking' };
    try {
      const update = await api.checkForUpdate();
      phase = update ? { kind: 'available', update } : { kind: 'current' };
    } catch (e) {
      phase = { kind: 'error', message: errorMessage(e), retry: check };
    }
  }

  async function install() {
    phase = { kind: 'downloading', received: 0, total: null };
    try {
      await api.installUpdate((p) => {
        phase =
          p.kind === 'downloading'
            ? { kind: 'downloading', received: p.received, total: p.total }
            : { kind: 'installing' };
      });
      phase = { kind: 'ready' };
    } catch (e) {
      phase = { kind: 'error', message: errorMessage(e), retry: check };
    }
  }

  const percent = $derived(
    phase.kind === 'downloading' && phase.total ? Math.min(100, Math.round((phase.received / phase.total) * 100)) : null,
  );
  const notes = $derived(
    phase.kind === 'available'
      ? (phase.update.notes ?? '')
          .split(/\r?\n/)
          .map((l) => l.replace(/^[-*]\s+/, '').trim())
          .filter(Boolean)
      : [],
  );

  onMount(check);
</script>

<Modal title={t('updates')} onclose={busy ? undefined : onclose} width={460}>
  {#if phase.kind === 'checking'}
    <p>{t('checkingUpdates')}</p>
  {:else if phase.kind === 'current'}
    <p>{t('upToDate', { version: __APP_VERSION__ })}</p>
  {:else if phase.kind === 'available'}
    <p>{t('updateAvailable', { version: phase.update.version, current: __APP_VERSION__ })}</p>
    {#if notes.length}
      <ul>
        {#each notes as item (item)}<li>{item}</li>{/each}
      </ul>
    {/if}
  {:else if phase.kind === 'downloading'}
    <p>{t('downloadingUpdate', { percent: percent === null ? '' : `${percent}%` })}</p>
    <progress max="100" value={percent ?? undefined}></progress>
  {:else if phase.kind === 'installing'}
    <p>{t('installingUpdate')}</p>
  {:else if phase.kind === 'ready'}
    <p>{t('updateReady')}</p>
    {#if activeTransfers > 0}<p class="warn">{t('updateTransfersWarning')}</p>{/if}
  {:else}
    <p class="warn" role="alert">{phase.message}</p>
  {/if}

  {#snippet footer()}
    {#if phase.kind === 'available'}
      <button type="button" class="btn" onclick={onclose}>{t('later')}</button>
      <button type="button" class="btn primary" data-autofocus onclick={install}>{t('downloadUpdate')}</button>
    {:else if phase.kind === 'ready'}
      <button type="button" class="btn" onclick={onclose}>{t('later')}</button>
      <button type="button" class="btn primary" data-autofocus onclick={() => api.restartApp()}>{t('restartNow')}</button>
    {:else if phase.kind === 'error'}
      <button type="button" class="btn" onclick={onclose}>{t('close')}</button>
      <button type="button" class="btn primary" data-autofocus onclick={phase.retry}>{t('tryAgain')}</button>
    {:else}
      <button type="button" class="btn" disabled={busy} onclick={onclose}>{t('close')}</button>
    {/if}
  {/snippet}
</Modal>

<style>
  p {
    margin: 0 0 8px;
  }
  ul {
    margin: 0;
    padding-left: 18px;
    max-height: 200px;
    overflow: auto;
  }
  progress {
    width: 100%;
  }
  .warn {
    color: var(--danger, inherit);
  }
</style>
