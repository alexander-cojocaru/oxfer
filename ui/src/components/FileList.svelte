<script lang="ts">
  import Icon from './Icon.svelte';
  import { formatBytes, formatDate } from '../lib/format';
  import type { SortKey, SortSpec } from '../lib/files';
  import {
    click,
    moveFocus,
    selectAll,
    toggleFocused,
    type Move,
    type Selection,
  } from '../lib/selection';
  import { t } from '../lib/i18n';
  import type { Entry } from '../lib/types';

  const ROW = 28;
  const OVERSCAN = 8;
  /** Used when the viewport has no measured height yet (e.g. in tests). */
  const FALLBACK_HEIGHT = 600;

  let {
    entries,
    selection,
    sort,
    label,
    loading = false,
    emptyText,
    onselectionchange,
    onsort,
    onopen,
    onup,
    oncontextmenu,
    ondragstartrow,
    ondropitems,
    onkeyaction,
  }: {
    entries: Entry[];
    selection: Selection;
    sort: SortSpec;
    label: string;
    loading?: boolean;
    emptyText: string;
    onselectionchange: (s: Selection) => void;
    onsort: (key: SortKey) => void;
    onopen: (e: Entry) => void;
    onup?: () => void;
    oncontextmenu?: (ev: MouseEvent, entry: Entry | null) => void;
    ondragstartrow?: (ev: DragEvent, entry: Entry) => void;
    ondropitems?: (ev: DragEvent, target: Entry | null) => void;
    onkeyaction?: (action: 'delete' | 'rename' | 'properties' | 'copy') => void;
  } = $props();

  const listId = `fl-${Math.random().toString(36).slice(2, 9)}`;
  let viewport: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let height = $state(0);
  let dropTarget = $state<string | null>(null);
  let dragOver = $state(false);

  const keys = $derived(entries.map((e) => e.path));
  const viewH = $derived(height || FALLBACK_HEIGHT);
  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - OVERSCAN));
  const last = $derived(Math.min(entries.length, Math.ceil((scrollTop + viewH) / ROW) + OVERSCAN));
  const visible = $derived(entries.slice(first, last));
  const pageRows = $derived(Math.max(1, Math.floor(viewH / ROW) - 1));
  const focusIndex = $derived(selection.focus ? keys.indexOf(selection.focus) : -1);

  function rowId(i: number) {
    return `${listId}-r${i}`;
  }

  function ensureVisible(index: number) {
    if (!viewport || index < 0) return;
    const top = index * ROW;
    const h = viewport.clientHeight || FALLBACK_HEIGHT;
    if (top < viewport.scrollTop) viewport.scrollTop = top;
    else if (top + ROW > viewport.scrollTop + h) viewport.scrollTop = top + ROW - h;
    scrollTop = viewport.scrollTop;
  }

  function isToggle(e: MouseEvent | KeyboardEvent) {
    return e.ctrlKey || e.metaKey;
  }

  function onrowclick(e: MouseEvent, entry: Entry) {
    onselectionchange(click(selection, keys, entry.path, { shift: e.shiftKey, toggle: isToggle(e) }));
    viewport?.focus();
  }

  function onrowcontext(e: MouseEvent, entry: Entry) {
    e.preventDefault();
    e.stopPropagation();
    if (!selection.selected.has(entry.path)) onselectionchange(click(selection, keys, entry.path));
    oncontextmenu?.(e, entry);
  }

  function onbackgroundcontext(e: MouseEvent) {
    e.preventDefault();
    oncontextmenu?.(e, null);
  }

  function move(m: Move, e: KeyboardEvent) {
    const next = moveFocus(selection, keys, m, { shift: e.shiftKey, toggle: isToggle(e) });
    onselectionchange(next);
    ensureVisible(next.focus ? keys.indexOf(next.focus) : -1);
  }

  function onkeydown(e: KeyboardEvent) {
    const focused = focusIndex >= 0 ? entries[focusIndex] : undefined;
    switch (e.key) {
      case 'ArrowDown':
        move(1, e);
        break;
      case 'ArrowUp':
        move(-1, e);
        break;
      case 'PageDown':
        move(pageRows, e);
        break;
      case 'PageUp':
        move(-pageRows, e);
        break;
      case 'Home':
        move('home', e);
        break;
      case 'End':
        move('end', e);
        break;
      case ' ':
        onselectionchange(toggleFocused(selection));
        break;
      case 'Enter':
        if (e.altKey) onkeyaction?.('properties');
        else if (focused) onopen(focused);
        else return;
        break;
      case 'Backspace':
        onup?.();
        break;
      case 'Delete':
        onkeyaction?.('delete');
        break;
      case 'F2':
        onkeyaction?.('rename');
        break;
      case 'F5':
        onkeyaction?.('copy');
        break;
      case 'a':
      case 'A':
        if (!isToggle(e)) return;
        onselectionchange(selectAll(keys, selection));
        break;
      default:
        return;
    }
    e.preventDefault();
  }

  function sortLabel(key: SortKey): 'ascending' | 'descending' | 'none' {
    if (sort.key !== key) return 'none';
    return sort.dir === 'asc' ? 'ascending' : 'descending';
  }

  function hasOurData(e: DragEvent) {
    return e.dataTransfer?.types.includes('application/x-oxfer') ?? false;
  }

  function ondragover(e: DragEvent, target: Entry | null) {
    if (!hasOurData(e)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
    dragOver = true;
    dropTarget = target && target.kind === 'dir' ? target.path : null;
  }

  function ondrop(e: DragEvent, target: Entry | null) {
    if (!hasOurData(e)) return;
    e.preventDefault();
    e.stopPropagation();
    dragOver = false;
    dropTarget = null;
    ondropitems?.(e, target && target.kind === 'dir' ? target : null);
  }

  function icon(e: Entry) {
    return e.kind === 'dir' ? 'folder' : e.kind === 'symlink' ? 'link' : 'file';
  }
</script>

<div class="list" class:drag-over={dragOver}>
  <div class="header" role="row">
    {#each [['name', t('name')], ['size', t('size')], ['modified', t('modified')]] as [key, text] (key)}
      <span class="col {key}" role="columnheader" aria-sort={sortLabel(key as SortKey)}>
        <button type="button" onclick={() => onsort(key as SortKey)}>
          {text}
          {#if sort.key === key}
            <span class="arrow" aria-hidden="true">{sort.dir === 'asc' ? '▲' : '▼'}</span>
          {/if}
        </button>
      </span>
    {/each}
  </div>
  <div
    class="viewport"
    role="grid"
    aria-label={label}
    aria-multiselectable="true"
    aria-rowcount={entries.length}
    aria-busy={loading}
    aria-activedescendant={focusIndex >= 0 ? rowId(focusIndex) : undefined}
    tabindex="0"
    bind:this={viewport}
    bind:clientHeight={height}
    onscroll={() => (scrollTop = viewport?.scrollTop ?? 0)}
    {onkeydown}
    oncontextmenu={onbackgroundcontext}
    ondragover={(e) => ondragover(e, null)}
    ondragleave={(e) => {
      if (e.currentTarget === e.target) dragOver = false;
    }}
    ondrop={(e) => ondrop(e, null)}
  >
    <div class="spacer" style:height="{entries.length * ROW}px">
      {#each visible as entry, i (entry.path)}
        {@const index = first + i}
        <div
          id={rowId(index)}
          class="row"
          class:selected={selection.selected.has(entry.path)}
          class:focused={index === focusIndex}
          class:drop={dropTarget === entry.path}
          role="row"
          aria-rowindex={index + 1}
          aria-selected={selection.selected.has(entry.path)}
          tabindex="-1"
          draggable="true"
          style:transform="translateY({index * ROW}px)"
          onclick={(e) => onrowclick(e, entry)}
          ondblclick={() => onopen(entry)}
          oncontextmenu={(e) => onrowcontext(e, entry)}
          onkeydown={() => {}}
          ondragstart={(e) => {
            if (!selection.selected.has(entry.path)) onselectionchange(click(selection, keys, entry.path));
            ondragstartrow?.(e, entry);
          }}
          ondragover={(e) => ondragover(e, entry)}
          ondrop={(e) => ondrop(e, entry)}
        >
          <span class="cell name" role="gridcell">
            <span class="icon" class:dir={entry.kind === 'dir'}><Icon name={icon(entry)} /></span>
            <span class="text" title={entry.name}>{entry.name}</span>
          </span>
          <span class="cell size" role="gridcell">{entry.kind === 'dir' ? '' : formatBytes(entry.size)}</span>
          <span class="cell modified" role="gridcell">{formatDate(entry.modified)}</span>
        </div>
      {/each}
    </div>
    {#if !loading && entries.length === 0}
      <p class="empty">{emptyText}</p>
    {/if}
  </div>
</div>

<style>
  .list {
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
    border-radius: var(--radius-sm);
  }
  .header,
  .row {
    display: grid;
    grid-template-columns: minmax(120px, 1fr) 88px 150px;
    align-items: center;
  }
  .header {
    border-bottom: 1px solid var(--border);
    background: var(--bg-sunken);
  }
  .col {
    display: flex;
  }
  .col button {
    flex: 1;
    background: none;
    border: none;
    text-align: left;
    padding: 6px 10px;
    font-size: 12px;
    font-weight: 600;
    color: var(--fg-muted);
    display: flex;
    gap: 4px;
    align-items: center;
  }
  .col.size button {
    justify-content: flex-end;
  }
  .col button:hover {
    color: var(--fg);
  }
  .arrow {
    font-size: 8px;
  }
  .viewport {
    position: relative;
    flex: 1;
    overflow: auto;
    min-height: 0;
    outline: none;
  }
  .viewport:focus-visible {
    box-shadow: inset 0 0 0 2px var(--focus);
  }
  .drag-over .viewport {
    background: var(--drop);
  }
  .spacer {
    position: relative;
    min-height: 100%;
  }
  .row {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: 28px;
    cursor: default;
    border-radius: 4px;
  }
  .row:hover {
    background: var(--bg-hover);
  }
  .row.selected {
    background: var(--bg-selected);
  }
  .viewport:focus .row.selected {
    background: var(--bg-selected-strong);
  }
  .viewport:focus .row.focused {
    box-shadow: inset 0 0 0 1px var(--focus);
  }
  .row.drop {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .cell {
    padding: 0 10px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .cell.name {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .text {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .icon {
    display: inline-flex;
    color: var(--fg-faint);
    flex: none;
  }
  .icon.dir {
    color: var(--accent);
  }
  .cell.size {
    text-align: right;
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  .cell.modified {
    color: var(--fg-muted);
    font-variant-numeric: tabular-nums;
  }
  .empty {
    position: absolute;
    inset: 40px 0 auto;
    text-align: center;
    color: var(--fg-faint);
    margin: 0;
  }
  @media (max-width: 1000px) {
    .header,
    .row {
      grid-template-columns: minmax(100px, 1fr) 72px;
    }
    .col.modified,
    .cell.modified {
      display: none;
    }
  }
</style>
