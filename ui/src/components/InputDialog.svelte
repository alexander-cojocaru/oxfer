<script lang="ts">
  import { onMount } from 'svelte';
  import Modal from './Modal.svelte';
  import { t, type MessageKey } from '../lib/i18n';
  import { validateName } from '../lib/files';

  let {
    title,
    label,
    initial = '',
    onsubmit,
    oncancel,
  }: {
    title: string;
    label: string;
    initial?: string;
    onsubmit: (value: string) => void;
    oncancel: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  let value = $state(initial);
  let input: HTMLInputElement | undefined = $state();
  const error = $derived(validateName(value));

  onMount(() => {
    // select the name without its extension
    const dot = initial.lastIndexOf('.');
    input?.setSelectionRange(0, dot > 0 ? dot : initial.length);
  });

  function submit(e: Event) {
    e.preventDefault();
    if (!error) onsubmit(value.trim());
  }
</script>

<Modal {title} onclose={oncancel}>
  <form id="input-dialog" onsubmit={submit}>
    <label class="field">
      <span>{label}</span>
      <input
        bind:this={input}
        bind:value
        data-autofocus
        spellcheck="false"
        autocomplete="off"
        aria-invalid={value !== '' && !!error}
        aria-describedby="input-dialog-error"
      />
    </label>
    <p id="input-dialog-error" class="error" aria-live="polite">
      {value !== '' && error ? t(error as MessageKey) : ''}
    </p>
  </form>
  {#snippet footer()}
    <button type="button" class="btn" onclick={oncancel}>{t('cancel')}</button>
    <button type="submit" form="input-dialog" class="btn primary" disabled={!!error}>{t('ok')}</button>
  {/snippet}
</Modal>

<style>
  .error {
    color: var(--danger);
    font-size: 12px;
    min-height: 16px;
    margin: 6px 0 0;
  }
</style>
