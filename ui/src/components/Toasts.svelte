<script lang="ts">
  import Icon from './Icon.svelte';
  import { app } from '../lib/state.svelte';
  import { t } from '../lib/i18n';
</script>

<div class="toasts" role="region" aria-label="Notifications">
  {#each app.toasts as toast (toast.id)}
    <div class="toast {toast.kind}" role={toast.kind === 'error' ? 'alert' : 'status'}>
      <Icon name={toast.kind === 'error' ? 'warning' : toast.kind === 'success' ? 'check' : 'info'} />
      <span class="msg">{toast.message}</span>
      <button type="button" class="btn icon ghost" aria-label={t('dismiss')} onclick={() => app.dismissToast(toast.id)}>
        <Icon name="x" size={14} />
      </button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 16px;
    bottom: 40px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    z-index: 200;
    max-width: min(420px, calc(100vw - 32px));
  }
  .toast {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 8px 10px 12px;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-left: 4px solid var(--accent);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
    animation: slide 0.15s ease-out;
  }
  .toast.error {
    border-left-color: var(--danger);
  }
  .toast > :global(svg) {
    flex: none;
    margin-top: 1px;
    color: var(--accent);
  }
  .toast.error > :global(svg) {
    color: var(--danger);
  }
  .toast.success > :global(svg) {
    color: var(--ok);
  }
  .toast.success {
    border-left-color: var(--ok);
  }
  .msg {
    flex: 1;
    color: var(--fg);
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
  }
  .btn {
    min-height: 22px;
    height: 22px;
    width: 22px;
  }
  @keyframes slide {
    from {
      transform: translateY(8px);
      opacity: 0;
    }
  }
</style>
