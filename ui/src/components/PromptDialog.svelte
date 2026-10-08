<script lang="ts">
  import Modal from './Modal.svelte';
  import Icon from './Icon.svelte';
  import { t } from '../lib/i18n';
  import { formatBytes, formatDate } from '../lib/format';
  import type { Prompt, PromptAnswer } from '../lib/types';

  let {
    id,
    prompt,
    onanswer,
  }: { id: number; prompt: Prompt; onanswer: (id: number, answer: PromptAnswer) => void } = $props();

  let secret = $state('');
  let applyToAll = $state(false);

  function answer(a: PromptAnswer) {
    onanswer(id, a);
    secret = '';
  }

  function cancel() {
    if (prompt.kind === 'hostKey') answer({ kind: 'hostKey', decision: 'reject' });
    else answer({ kind: 'cancel' });
  }
</script>

{#if prompt.kind === 'hostKey'}
  <Modal title={t('hostKeyTitle')} onclose={cancel} role="alertdialog" width={540}>
    <div class="row">
      <span class="badge warn"><Icon name="shield" size={20} /></span>
      <div>
        <p>{t('hostKeyIntro', { host: `${prompt.host}:${prompt.port}` })}</p>
        <dl>
          <dt>{t('algorithm')}</dt>
          <dd class="mono">{prompt.algorithm}</dd>
          <dt>{t('fingerprint')}</dt>
          <dd class="mono fingerprint" data-testid="fingerprint">{prompt.fingerprint}</dd>
        </dl>
        <p class="hint">{t('hostKeyVerify')}</p>
      </div>
    </div>
    {#snippet footer()}
      <button type="button" class="btn" data-autofocus onclick={() => answer({ kind: 'hostKey', decision: 'reject' })}>
        {t('reject')}
      </button>
      <button type="button" class="btn" onclick={() => answer({ kind: 'hostKey', decision: 'acceptOnce' })}>
        {t('acceptOnce')}
      </button>
      <button type="button" class="btn primary" onclick={() => answer({ kind: 'hostKey', decision: 'acceptAndSave' })}>
        {t('acceptAndSave')}
      </button>
    {/snippet}
  </Modal>
{:else if prompt.kind === 'password'}
  <Modal title={prompt.secret === 'password' ? t('passwordTitle') : t('passphraseTitle')} onclose={cancel}>
    <form
      id="prompt-secret"
      onsubmit={(e) => {
        e.preventDefault();
        answer({ kind: 'password', value: secret });
      }}
    >
      {#if prompt.retry}
        <p class="error" role="alert">{t('wrongPassword')}</p>
      {/if}
      <label class="field">
        <span>
          {prompt.secret === 'password'
            ? t('passwordFor', { user: prompt.user, host: prompt.host })
            : t('passphraseFor', { user: prompt.user })}
        </span>
        <input type="password" bind:value={secret} data-autofocus autocomplete="off" />
      </label>
    </form>
    {#snippet footer()}
      <button type="button" class="btn" onclick={cancel}>{t('cancel')}</button>
      <button type="submit" form="prompt-secret" class="btn primary">{t('ok')}</button>
    {/snippet}
  </Modal>
{:else if prompt.kind === 'confirm'}
  <Modal title={prompt.title} onclose={cancel} role="alertdialog">
    <div class="row">
      {#if prompt.danger}<span class="badge warn"><Icon name="warning" size={20} /></span>{/if}
      <p>{prompt.message}</p>
    </div>
    {#snippet footer()}
      <button type="button" class="btn" data-autofocus onclick={() => answer({ kind: 'confirm', ok: false })}>
        {t('cancel')}
      </button>
      <button
        type="button"
        class="btn {prompt.kind === 'confirm' && prompt.danger ? 'danger' : 'primary'}"
        onclick={() => answer({ kind: 'confirm', ok: true })}>{t('connect')}</button
      >
    {/snippet}
  </Modal>
{:else if prompt.kind === 'conflict'}
  <Modal title={t('conflictTitle')} onclose={cancel} width={520}>
    <p>{t('conflictMessage', { name: prompt.dst.name })}</p>
    <table>
      <thead>
        <tr><th></th><th>{t('size')}</th><th>{t('modified')}</th></tr>
      </thead>
      <tbody>
        <tr>
          <th scope="row">{t('source')}</th>
          <td>{formatBytes(prompt.src.size)}</td>
          <td>{formatDate(prompt.src.modified)}</td>
        </tr>
        <tr>
          <th scope="row">{t('destination')}</th>
          <td>{formatBytes(prompt.dst.size)}</td>
          <td>{formatDate(prompt.dst.modified)}</td>
        </tr>
      </tbody>
    </table>
    <p class="mono path">{prompt.dst.path}</p>
    <label class="check">
      <input type="checkbox" bind:checked={applyToAll} />
      {t('applyToAll')}
    </label>
    {#snippet footer()}
      <button type="button" class="btn" data-autofocus onclick={() => answer({ kind: 'conflict', action: 'skip', applyToAll })}>
        {t('skip')}
      </button>
      <button
        type="button"
        class="btn primary"
        onclick={() => answer({ kind: 'conflict', action: 'overwrite', applyToAll })}>{t('overwrite')}</button
      >
    {/snippet}
  </Modal>
{/if}

<style>
  p {
    margin: 0 0 10px;
    overflow-wrap: anywhere;
  }
  .row {
    display: flex;
    gap: 14px;
    align-items: flex-start;
  }
  .badge {
    flex: none;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 50%;
  }
  .badge.warn {
    background: var(--warn-soft);
    color: var(--warn);
  }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 12px;
    margin: 0 0 10px;
  }
  dt {
    color: var(--fg-muted);
  }
  dd {
    margin: 0;
  }
  .fingerprint {
    overflow-wrap: anywhere;
    user-select: all;
    -webkit-user-select: all;
    padding: 4px 6px;
    background: var(--bg-sunken);
    border-radius: 4px;
  }
  .error {
    color: var(--danger);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    margin-bottom: 8px;
  }
  th,
  td {
    text-align: left;
    padding: 4px 6px;
    border-bottom: 1px solid var(--border);
  }
  thead th {
    color: var(--fg-muted);
    font-weight: 600;
  }
  .path {
    color: var(--fg-muted);
  }
  .check {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .check input {
    min-height: auto;
  }
</style>
