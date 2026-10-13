<script lang="ts">
  import Modal from './Modal.svelte';
  import { api, errorMessage } from '../lib/api';
  import { formatBytes } from '../lib/format';
  import { t } from '../lib/i18n';
  import { app } from '../lib/state.svelte';
  import type { Entry, FilePreview, SessionId } from '../lib/types';

  let {
    session,
    entry,
    oneditor,
    onclose,
  }: {
    session: SessionId;
    entry: Entry;
    /** Open the file in the text editor. */
    oneditor: (entry: Entry) => void;
    onclose: () => void;
  } = $props();

  let preview = $state<FilePreview | null>(null);
  let error = $state<string | null>(null);
  let wrap = $state(true);

  $effect(() => {
    let stale = false;
    preview = null;
    error = null;
    api
      .readPreview(session, entry.path)
      .then((p) => {
        if (!stale) preview = p;
      })
      .catch((e) => {
        if (!stale) error = errorMessage(e);
      });
    return () => {
      stale = true;
    };
  });

  async function copy() {
    const text = preview?.content;
    if (text == null) return;
    try {
      await navigator.clipboard.writeText(text);
      app.toast('success', t('copiedToClipboard'));
    } catch (e) {
      app.error(e);
    }
  }

  const lineCount = $derived(preview?.content ? preview.content.split('\n').length : 0);
</script>

<Modal title={entry.name} {onclose} width={880}>
  <div class="meta">
    <span class="mono path" title={entry.path}>{entry.path}</span>
    <span>{formatBytes(entry.size)}</span>
    {#if preview?.content != null}
      <span>{t('lines', { count: lineCount })}</span>
      <label class="check">
        <input type="checkbox" bind:checked={wrap} />
        {t('wrapLines')}
      </label>
    {/if}
  </div>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {:else if !preview}
    <p class="muted" aria-live="polite">{t('loading')}</p>
  {:else if preview.binary}
    <p class="muted">{t('previewBinary')}</p>
  {:else}
    {#if preview.truncated}
      <p class="note" role="note">{t('previewTruncated', { size: formatBytes(preview.content?.length ?? 0) })}</p>
    {/if}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <pre class="content mono" class:wrap tabindex="0" aria-label={t('previewOf', { name: entry.name })}>{preview.content}</pre>
  {/if}
  {#snippet footer()}
    {#if preview?.content != null}
      <button type="button" class="btn" onclick={copy}>{t('copyToClipboard')}</button>
    {/if}
    <button type="button" class="btn" onclick={() => oneditor(entry)} title="F4">{t('openInEditor')}</button>
    <button type="button" class="btn primary" data-autofocus onclick={onclose}>{t('close')}</button>
  {/snippet}
</Modal>

<style>
  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 16px;
    margin-bottom: 8px;
    color: var(--fg-muted);
    font-size: 12px;
  }
  .path {
    flex: 1 1 200px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .check {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .check input {
    min-height: auto;
  }
  .content {
    margin: 0;
    max-height: min(65vh, 640px);
    overflow: auto;
    padding: 10px 12px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    font-size: 12px;
    line-height: 1.5;
    white-space: pre;
    tab-size: 4;
    user-select: text;
    -webkit-user-select: text;
  }
  .content.wrap {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .muted {
    color: var(--fg-muted);
  }
  .note {
    margin: 0 0 8px;
    padding: 6px 10px;
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
</style>
