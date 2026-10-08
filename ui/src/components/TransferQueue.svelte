<script lang="ts">
  import Icon from './Icon.svelte';
  import { api } from '../lib/api';
  import { app } from '../lib/state.svelte';
  import { formatBytes, formatDuration, formatSpeed } from '../lib/format';
  import { t, type MessageKey } from '../lib/i18n';
  import { baseName } from '../lib/files';
  import { canRetry, etaSeconds, fraction, isActive, type QueueState, type TransferView } from '../lib/transfers';

  let { queue }: { queue: QueueState } = $props();

  let expanded = $state<Record<number, boolean>>({});

  const transfers = $derived(queue.order.map((id) => queue.byId[id]));

  const STATUS: Record<TransferView['status'], MessageKey> = {
    planning: 'statusPlanning',
    running: 'statusRunning',
    paused: 'statusPaused',
    done: 'statusDone',
    failed: 'statusFailed',
    cancelled: 'statusCancelled',
  };

  function title(tr: TransferView) {
    const names = tr.sources.map(baseName);
    const what = names.length === 1 ? names[0] : `${names[0]} +${names.length - 1}`;
    return `${what} → ${tr.destDir}`;
  }

  async function run(fn: () => Promise<unknown>) {
    try {
      await fn();
    } catch (e) {
      app.error(e);
    }
  }
</script>

<div class="queue">
  {#if transfers.length === 0}
    <p class="empty">{t('noTransfers')}</p>
  {:else}
    <ul aria-label={t('transfers')}>
      {#each transfers as tr (tr.id)}
        {@const pct = Math.round(fraction(tr) * 100)}
        <li class="item {tr.status}" aria-label={title(tr)}>
          <div class="line">
            <span class="title" title={title(tr)}>{title(tr)}</span>
            <span class="status">{t(STATUS[tr.status])}</span>
            <div class="actions">
              {#if isActive(tr)}
                {#if tr.paused}
                  <button type="button" class="btn icon ghost" aria-label={t('resume')} title={t('resume')} onclick={() => run(() => api.resumeTransfer(tr.id))}>
                    <Icon name="play" />
                  </button>
                {:else}
                  <button type="button" class="btn icon ghost" aria-label={t('pause')} title={t('pause')} onclick={() => run(() => api.pauseTransfer(tr.id))}>
                    <Icon name="pause" />
                  </button>
                {/if}
                <button type="button" class="btn icon ghost" aria-label={t('cancel')} title={t('cancel')} onclick={() => run(() => api.cancelTransfer(tr.id))}>
                  <Icon name="x" />
                </button>
              {:else if canRetry(tr)}
                <button type="button" class="btn icon ghost" aria-label={t('retry')} title={t('retry')} onclick={() => run(() => api.retryTransfer(tr.id))}>
                  <Icon name="retry" />
                </button>
              {/if}
              {#if tr.log.length > 0 || Object.keys(tr.active).length > 0}
                <button
                  type="button"
                  class="btn icon ghost"
                  aria-expanded={!!expanded[tr.id]}
                  aria-label={expanded[tr.id] ? t('hideDetails') : t('showDetails')}
                  title={expanded[tr.id] ? t('hideDetails') : t('showDetails')}
                  onclick={() => (expanded = { ...expanded, [tr.id]: !expanded[tr.id] })}
                >
                  <Icon name={expanded[tr.id] ? 'chevronDown' : 'chevronRight'} />
                </button>
              {/if}
            </div>
          </div>
          <div
            class="bar"
            role="progressbar"
            aria-label={title(tr)}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={pct}
          >
            <div class="fill" style:width="{pct}%"></div>
          </div>
          <div class="meta">
            <span>
              {t('transferSummary', {
                done: tr.completedFiles + tr.skippedFiles,
                files: tr.files,
                bytes: `${formatBytes(tr.bytesDone)} / ${formatBytes(tr.bytes)}`,
              })}
            </span>
            {#if isActive(tr) && !tr.paused}
              <span>{formatSpeed(tr.speed)}</span>
              <span>{t('eta', { eta: formatDuration(etaSeconds(tr)) })}</span>
            {/if}
            {#if tr.skippedFiles}<span class="skipped">{t('skippedLabel')}: {tr.skippedFiles}</span>{/if}
            {#if tr.failedFiles}<span class="failed">{t('failedLabel')}: {tr.failedFiles}</span>{/if}
          </div>
          {#if expanded[tr.id]}
            <div class="details">
              {#each Object.values(tr.active) as job (job.jobId)}
                {@const jp = job.size ? Math.round((job.done / job.size) * 100) : 0}
                <div class="job">
                  <span class="mono name" title={job.src}>{baseName(job.src)}</span>
                  <div class="bar small" role="progressbar" aria-label={baseName(job.src)} aria-valuemin={0} aria-valuemax={100} aria-valuenow={jp}>
                    <div class="fill" style:width="{jp}%"></div>
                  </div>
                  <span class="num">{jp}%</span>
                  {#if job.message}<span class="msg">{job.message}</span>{/if}
                </div>
              {/each}
              {#if tr.log.length}
                <ul class="log" aria-label="log">
                  {#each tr.log as item, i (i)}
                    <li class={item.kind}>
                      <strong>{item.kind === 'failed' ? t('failedLabel') : t('skippedLabel')}</strong>
                      <span class="mono">{item.path}</span>
                      <span>{item.message}</span>
                    </li>
                  {/each}
                </ul>
              {/if}
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .queue {
    height: 100%;
    overflow: auto;
  }
  .empty {
    margin: 0;
    padding: 16px;
    color: var(--fg-faint);
    text-align: center;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .item {
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
  }
  .line {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }
  .status {
    font-size: 12px;
    color: var(--fg-muted);
  }
  .failed .status,
  span.failed {
    color: var(--danger);
  }
  .done .status {
    color: var(--ok);
  }
  .actions {
    display: flex;
    gap: 2px;
  }
  .actions .btn {
    min-height: 24px;
    height: 24px;
    width: 24px;
    padding: 2px;
  }
  .bar {
    height: 6px;
    border-radius: 3px;
    background: var(--bg-sunken);
    overflow: hidden;
    margin: 6px 0 4px;
  }
  .bar.small {
    height: 4px;
    margin: 0;
    flex: 1;
  }
  .fill {
    height: 100%;
    background: linear-gradient(90deg, var(--accent), var(--accent-2));
    transition: width 0.2s linear;
  }
  .failed .fill {
    background: var(--danger);
  }
  .paused .fill,
  .cancelled .fill {
    background: var(--fg-faint);
  }
  .meta {
    display: flex;
    gap: 14px;
    flex-wrap: wrap;
    font-size: 12px;
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  span.skipped {
    color: var(--warn);
  }
  .details {
    margin-top: 6px;
    padding: 6px 8px;
    background: var(--bg-sunken);
    border-radius: var(--radius-sm);
  }
  .job {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 2px 0;
    font-size: 12px;
  }
  .job .name {
    width: 40%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .num {
    width: 36px;
    text-align: right;
  }
  .msg {
    color: var(--warn);
  }
  .log li {
    display: flex;
    gap: 8px;
    font-size: 12px;
    padding: 2px 0;
    flex-wrap: wrap;
  }
  .log li.failed strong {
    color: var(--danger);
  }
  .log li.skipped strong {
    color: var(--warn);
  }
  .log .mono {
    overflow-wrap: anywhere;
  }
</style>
