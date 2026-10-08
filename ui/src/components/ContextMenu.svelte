<script lang="ts" module>
  export interface MenuItem {
    label: string;
    action: () => void;
    disabled?: boolean;
    danger?: boolean;
    shortcut?: string;
  }
  export type MenuEntry = MenuItem | 'separator';
</script>

<script lang="ts">
  import { onMount } from 'svelte';

  let { x, y, items, onclose }: { x: number; y: number; items: MenuEntry[]; onclose: () => void } = $props();

  let menu: HTMLDivElement | undefined = $state();
  let left = $state(0);
  let top = $state(0);

  onMount(() => {
    // keep inside the window
    const r = menu?.getBoundingClientRect();
    const w = r?.width ?? 200;
    const h = r?.height ?? 200;
    left = Math.max(4, Math.min(x, window.innerWidth - w - 4));
    top = Math.max(4, Math.min(y, window.innerHeight - h - 4));
    buttons()[0]?.focus();
    const away = (e: MouseEvent) => {
      if (menu && !menu.contains(e.target as Node)) onclose();
    };
    window.addEventListener('mousedown', away, true);
    window.addEventListener('blur', onclose);
    return () => {
      window.removeEventListener('mousedown', away, true);
      window.removeEventListener('blur', onclose);
    };
  });

  function buttons() {
    return [...(menu?.querySelectorAll<HTMLButtonElement>('button:not([disabled])') ?? [])];
  }

  function onkeydown(e: KeyboardEvent) {
    const list = buttons();
    const i = list.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === 'ArrowDown') list[(i + 1) % list.length]?.focus();
    else if (e.key === 'ArrowUp') list[(i - 1 + list.length) % list.length]?.focus();
    else if (e.key === 'Home') list[0]?.focus();
    else if (e.key === 'End') list[list.length - 1]?.focus();
    else if (e.key === 'Escape' || e.key === 'Tab') onclose();
    else return;
    e.preventDefault();
    e.stopPropagation();
  }

  function run(item: MenuItem) {
    onclose();
    item.action();
  }
</script>

<div class="menu" role="menu" tabindex="-1" bind:this={menu} style:left="{left}px" style:top="{top}px" {onkeydown}>
  {#each items as item, i (i)}
    {#if item === 'separator'}
      <div class="sep" role="separator"></div>
    {:else}
      <button
        type="button"
        role="menuitem"
        class:danger={item.danger}
        disabled={item.disabled}
        onclick={() => run(item)}
      >
        <span>{item.label}</span>
        {#if item.shortcut}<kbd>{item.shortcut}</kbd>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 90;
    min-width: 200px;
    padding: 4px;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
    outline: none;
  }
  button {
    display: flex;
    width: 100%;
    justify-content: space-between;
    gap: 16px;
    background: none;
    border: none;
    text-align: left;
    padding: 5px 10px;
    border-radius: 4px;
  }
  button:hover:not(:disabled),
  button:focus-visible {
    background: var(--accent);
    color: var(--accent-fg);
    outline: none;
  }
  button:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .danger {
    color: var(--danger);
  }
  kbd {
    font-family: var(--font);
    font-size: 11px;
    opacity: 0.7;
  }
  .sep {
    height: 1px;
    background: var(--border);
    margin: 4px 6px;
  }
</style>
