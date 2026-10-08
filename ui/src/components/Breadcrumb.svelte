<script lang="ts">
  import { tick } from 'svelte';
  import { breadcrumbs } from '../lib/files';
  import { t } from '../lib/i18n';

  let { path, onnavigate }: { path: string; onnavigate: (path: string) => void } = $props();

  let editing = $state(false);
  let draft = $state('');
  let input: HTMLInputElement | undefined = $state();
  const crumbs = $derived(breadcrumbs(path));

  export async function edit() {
    draft = path;
    editing = true;
    await tick();
    input?.focus();
    input?.select();
  }

  function commit() {
    editing = false;
    const p = draft.trim();
    if (p && p !== path) onnavigate(p);
  }
</script>

<div class="bar">
  {#if editing}
    <input
      bind:this={input}
      bind:value={draft}
      class="mono"
      aria-label={t('path')}
      spellcheck="false"
      autocomplete="off"
      onkeydown={(e) => {
        if (e.key === 'Enter') commit();
        else if (e.key === 'Escape') {
          e.stopPropagation();
          editing = false;
        }
      }}
      onblur={() => (editing = false)}
    />
  {:else}
    <nav aria-label={t('path')}>
      <ol>
        {#each crumbs as c, i (c.path)}
          <li>
            {#if i > 0 && crumbs[i - 1].label !== '/'}<span class="sep" aria-hidden="true">/</span>{/if}
            <button
              type="button"
              class="crumb"
              aria-current={i === crumbs.length - 1 ? 'location' : undefined}
              onclick={() => onnavigate(c.path)}>{c.label}</button
            >
          </li>
        {/each}
      </ol>
      <button type="button" class="edit" aria-label={t('editPath')} title="{t('editPath')} (Ctrl+L)" onclick={edit}
      ></button>
    </nav>
  {/if}
</div>

<style>
  .bar {
    flex: 1;
    min-width: 0;
    display: flex;
  }
  input {
    flex: 1;
    min-width: 0;
  }
  nav {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-elev);
    min-height: 28px;
    overflow: hidden;
  }
  ol {
    display: flex;
    list-style: none;
    margin: 0;
    padding: 0 4px;
    overflow: hidden;
    white-space: nowrap;
    flex: none;
    max-width: 100%;
    /* long paths overflow (and are clipped) at the start: the end stays visible */
    justify-content: flex-end;
  }
  li {
    display: inline-flex;
    align-items: center;
  }
  .sep {
    color: var(--fg-faint);
    padding: 0 1px;
  }
  .crumb {
    background: none;
    border: none;
    padding: 2px 4px;
    border-radius: 4px;
    color: var(--fg-muted);
  }
  .crumb:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }
  .crumb[aria-current='location'] {
    color: var(--fg);
    font-weight: 600;
  }
  .edit {
    flex: 1;
    align-self: stretch;
    min-width: 24px;
    background: none;
    border: none;
    cursor: text;
  }
</style>
