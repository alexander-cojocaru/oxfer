<script lang="ts">
  import Modal from './Modal.svelte';
  import { t } from '../lib/i18n';

  let {
    title,
    message,
    confirmLabel,
    danger = false,
    onconfirm,
    oncancel,
  }: {
    title: string;
    message: string;
    confirmLabel: string;
    danger?: boolean;
    onconfirm: () => void;
    oncancel: () => void;
  } = $props();
</script>

<Modal {title} onclose={oncancel} role="alertdialog">
  <p class="msg">{message}</p>
  {#snippet footer()}
    <button type="button" class="btn" data-autofocus onclick={oncancel}>{t('cancel')}</button>
    <button type="button" class="btn {danger ? 'danger' : 'primary'}" onclick={onconfirm}>{confirmLabel}</button>
  {/snippet}
</Modal>

<style>
  .msg {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>
