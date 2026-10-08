<script lang="ts">
  import Modal from './Modal.svelte';
  import { t } from '../lib/i18n';
  import { formatBytes, formatDate, formatOctal, formatPermissions } from '../lib/format';
  import type { Entry } from '../lib/types';

  let { entry, onclose }: { entry: Entry; onclose: () => void } = $props();
</script>

<Modal title="{t('properties')}: {entry.name}" {onclose} width={480}>
  <dl>
    <dt>{t('name')}</dt>
    <dd>{entry.name}</dd>
    <dt>{t('path')}</dt>
    <dd class="mono">{entry.path}</dd>
    <dt>{t('kind')}</dt>
    <dd>{entry.kind}</dd>
    {#if entry.kind !== 'dir'}
      <dt>{t('size')}</dt>
      <dd>{formatBytes(entry.size)} ({entry.size.toLocaleString()} B)</dd>
    {/if}
    <dt>{t('modified')}</dt>
    <dd>{formatDate(entry.modified)}</dd>
    <dt>{t('permissions')}</dt>
    <dd class="mono">{formatPermissions(entry.permissions, entry.kind)} ({formatOctal(entry.permissions)})</dd>
  </dl>
  {#snippet footer()}
    <button type="button" class="btn primary" data-autofocus onclick={onclose}>{t('close')}</button>
  {/snippet}
</Modal>

<style>
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 6px 16px;
    margin: 0;
  }
  dt {
    color: var(--fg-muted);
  }
  dd {
    margin: 0;
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
  }
</style>
