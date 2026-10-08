<script lang="ts">
  import { onMount, type Snippet } from 'svelte';

  let {
    title,
    onclose,
    children,
    footer,
    width = 460,
    role = 'dialog',
  }: {
    title: string;
    onclose?: () => void;
    children: Snippet;
    footer?: Snippet;
    width?: number;
    role?: 'dialog' | 'alertdialog';
  } = $props();

  let dialog: HTMLDivElement | undefined = $state();
  const titleId = `dlg-${Math.random().toString(36).slice(2, 9)}`;

  const FOCUSABLE =
    'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

  onMount(() => {
    const previous = document.activeElement as HTMLElement | null;
    const target =
      dialog?.querySelector<HTMLElement>('[data-autofocus]') ?? dialog?.querySelector<HTMLElement>(FOCUSABLE);
    (target ?? dialog)?.focus();
    return () => previous?.focus?.();
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && onclose) {
      e.stopPropagation();
      e.preventDefault();
      onclose();
      return;
    }
    if (e.key !== 'Tab' || !dialog) return;
    const items = [...dialog.querySelectorAll<HTMLElement>(FOCUSABLE)];
    if (items.length === 0) return;
    const first = items[0];
    const last = items[items.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }
</script>

<div class="backdrop">
  <div
    class="dialog"
    {role}
    aria-modal="true"
    aria-labelledby={titleId}
    tabindex="-1"
    style:width="min({width}px, calc(100vw - 32px))"
    bind:this={dialog}
    {onkeydown}
  >
    <header>
      <h2 id={titleId}>{title}</h2>
    </header>
    <div class="body">
      {@render children()}
    </div>
    {#if footer}
      <footer>
        {@render footer()}
      </footer>
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(10 12 18 / 0.45);
    display: grid;
    place-items: center;
    z-index: 100;
    animation: fade 0.12s ease-out;
  }
  .dialog {
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: var(--shadow);
    max-height: calc(100vh - 48px);
    display: flex;
    flex-direction: column;
    outline: none;
    animation: pop 0.14s ease-out;
  }
  header {
    padding: 16px 20px 4px;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .body {
    padding: 12px 20px;
    overflow: auto;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 20px 16px;
    flex-wrap: wrap;
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @keyframes pop {
    from {
      transform: scale(0.97);
      opacity: 0;
    }
  }
</style>
