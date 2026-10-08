<script lang="ts" module>
  export interface DragPayload {
    session: string;
    paths: string[];
  }
  export const DRAG_TYPE = 'application/x-oxfer';

  export interface TransferIntent {
    fromSession: string;
    paths: string[];
    destDir: string;
  }
</script>

<script lang="ts">
  import Breadcrumb from './Breadcrumb.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import ContextMenu, { type MenuEntry } from './ContextMenu.svelte';
  import PreviewDialog from './PreviewDialog.svelte';
  import FileList from './FileList.svelte';
  import Icon from './Icon.svelte';
  import InputDialog from './InputDialog.svelte';
  import PropertiesDialog from './PropertiesDialog.svelte';
  import { api } from '../lib/api';
  import { app } from '../lib/state.svelte';
  import { baseName, filterEntries, joinPath, nextSort, parentPath, sortEntries, type SortKey, type SortSpec } from '../lib/files';
  import { emptySelection, prune, selectedInOrder, type Selection } from '../lib/selection';
  import { t } from '../lib/i18n';
  import type { Entry, SessionInfo } from '../lib/types';

  let {
    side,
    session,
    sessions,
    active,
    onactivate,
    onsessionchange,
    ontransfer,
    oncopy,
  }: {
    side: 'left' | 'right';
    session: SessionInfo | null;
    sessions: SessionInfo[];
    active: boolean;
    onactivate: () => void;
    onsessionchange: (id: string) => void;
    ontransfer: (intent: TransferIntent) => void;
    oncopy: () => void;
  } = $props();

  let path = $state('');
  let raw = $state<Entry[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let selection = $state<Selection>(emptySelection);
  let sort = $state<SortSpec>({ key: 'name', dir: 'asc' });
  let filter = $state('');
  let showHidden = $state(false);
  let menu = $state<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  let dialog = $state<
    | { kind: 'rename'; entry: Entry }
    | { kind: 'mkdir' }
    | { kind: 'delete'; paths: string[] }
    | { kind: 'properties'; entry: Entry }
    | { kind: 'preview'; entry: Entry }
    | null
  >(null);

  let breadcrumb: Breadcrumb | undefined = $state();
  let filterInput: HTMLInputElement | undefined = $state();
  let paneEl: HTMLElement | undefined = $state();

  const entries = $derived(sortEntries(filterEntries(raw, filter, showHidden), sort));
  const keys = $derived(entries.map((e) => e.path));
  const selectedEntries = $derived(entries.filter((e) => selection.selected.has(e.path)));
  const sessionId = $derived(session?.id ?? null);

  let loadSeq = 0;
  let lastSession: string | null = null;

  $effect(() => {
    // (re)open the home directory whenever the pane switches session
    const s = session;
    if (!s) {
      raw = [];
      path = '';
      lastSession = null;
      return;
    }
    if (s.id !== lastSession) {
      lastSession = s.id;
      void navigate(s.home);
    }
  });

  $effect(() => {
    // keep the selection in sync with what is visible
    const pruned = prune(selection, keys);
    if (pruned.selected.size !== selection.selected.size || pruned.focus !== selection.focus) selection = pruned;
  });

  export async function navigate(target: string, focusPath?: string) {
    if (!session) return;
    const id = session.id;
    const seq = ++loadSeq;
    loading = true;
    error = null;
    try {
      const list = await api.listDir(id, target);
      if (seq !== loadSeq) return;
      const changed = target !== path;
      raw = list;
      path = target;
      if (changed) {
        selection = focusPath ? { selected: new Set([focusPath]), anchor: focusPath, focus: focusPath } : emptySelection;
        if (!focusPath) filter = '';
      }
    } catch (e) {
      if (seq !== loadSeq) return;
      error = String((e as { message?: string })?.message ?? e);
      app.error(e);
    } finally {
      if (seq === loadSeq) loading = false;
    }
  }

  export function refresh() {
    if (path) void navigate(path);
  }

  export function currentPath() {
    return path;
  }

  export function selectedPaths(): string[] {
    return selectedInOrder(selection, keys);
  }

  export function focusList() {
    paneEl?.querySelector<HTMLElement>('[role="grid"]')?.focus();
  }

  export function focusFilter() {
    filterInput?.focus();
  }

  export function editPath() {
    breadcrumb?.edit();
  }

  export function toggleHidden() {
    showHidden = !showHidden;
  }

  export function goUp() {
    const p = parentPath(path);
    if (p) void navigate(p, path);
  }

  function open(e: Entry) {
    if (e.kind === 'dir' || e.kind === 'symlink') {
      if (e.kind === 'symlink') {
        // follow the link if it points to a directory
        api
          .stat(session!.id, e.path)
          .then((st) => {
            if (st.kind === 'dir') void navigate(e.path);
          })
          .catch((err) => app.error(err));
        return;
      }
      void navigate(e.path);
    } else if (e.kind === 'file') {
      dialog = { kind: 'preview', entry: e };
    } else {
      dialog = { kind: 'properties', entry: e };
    }
  }

  /** The focused entry, or the only selected one. */
  function currentFile(): Entry | undefined {
    const e = selectedEntries.length === 1 ? selectedEntries[0] : entries.find((x) => x.path === selection.focus);
    return e?.kind === 'file' ? e : undefined;
  }

  export function previewSelected() {
    const e = currentFile();
    if (e) dialog = { kind: 'preview', entry: e };
    else app.toast('info', t('noFileSelected'));
  }

  export async function editInEditor(e: Entry | undefined = currentFile()) {
    if (!e || !session) {
      app.toast('info', t('noFileSelected'));
      return;
    }
    try {
      await api.openInEditor(session.id, e.path);
      if (session.protocol !== 'local') app.toast('info', t('editingRemote', { name: e.name }), 8000);
    } catch (err) {
      app.error(err);
    }
  }

  export function newFolder() {
    if (session) dialog = { kind: 'mkdir' };
  }

  export function renameSelected() {
    const e = selectedEntries.length === 1 ? selectedEntries[0] : entries.find((x) => x.path === selection.focus);
    if (e) dialog = { kind: 'rename', entry: e };
    else app.toast('info', t('noSelection'));
  }

  export function deleteSelected() {
    const paths = selectedPaths();
    if (paths.length) dialog = { kind: 'delete', paths };
    else app.toast('info', t('noSelection'));
  }

  export function showProperties() {
    const e = selectedEntries[0] ?? entries.find((x) => x.path === selection.focus);
    if (e) dialog = { kind: 'properties', entry: e };
  }

  async function doMkdir(name: string) {
    dialog = null;
    const target = joinPath(path, name);
    try {
      await api.mkdir(session!.id, target);
      await navigate(path);
      selection = { selected: new Set([target]), anchor: target, focus: target };
    } catch (e) {
      app.error(e);
    }
  }

  async function doRename(entry: Entry, name: string) {
    dialog = null;
    if (name === entry.name) return;
    const target = joinPath(parentPath(entry.path) ?? path, name);
    try {
      await api.rename(session!.id, entry.path, target);
      await navigate(path);
      selection = { selected: new Set([target]), anchor: target, focus: target };
    } catch (e) {
      app.error(e);
    }
  }

  async function doDelete(paths: string[]) {
    dialog = null;
    try {
      await api.delete(session!.id, paths);
    } catch (e) {
      app.error(e);
    }
    await navigate(path);
    focusList();
  }

  function onsort(key: SortKey) {
    sort = nextSort(sort, key);
  }

  function oncontextmenu(ev: MouseEvent, entry: Entry | null) {
    onactivate();
    const has = entry !== null || selection.selected.size > 0;
    const items: MenuEntry[] = [];
    if (entry && (entry.kind === 'dir' || entry.kind === 'symlink')) {
      items.push({ label: t('open'), action: () => open(entry), shortcut: 'Enter' });
    }
    if (entry?.kind === 'file') {
      items.push(
        { label: t('preview'), action: () => (dialog = { kind: 'preview', entry }), shortcut: 'F3' },
        { label: t('openInEditor'), action: () => editInEditor(entry), shortcut: 'F4' },
        'separator',
      );
    }
    items.push(
      { label: t('copyToOther'), action: oncopy, disabled: !has, shortcut: 'F5' },
      'separator',
      { label: t('rename'), action: renameSelected, disabled: !entry, shortcut: 'F2' },
      { label: t('delete'), action: deleteSelected, disabled: !has, danger: true, shortcut: 'Del' },
      'separator',
      { label: t('newFolder'), action: newFolder, shortcut: 'F7' },
      { label: t('refresh'), action: refresh, shortcut: 'Ctrl+R' },
      'separator',
      { label: t('properties'), action: showProperties, disabled: !entry, shortcut: 'Alt+Enter' },
    );
    menu = { x: ev.clientX, y: ev.clientY, items };
  }

  function ondragstartrow(ev: DragEvent, entry: Entry) {
    if (!session || !ev.dataTransfer) return;
    const paths = selection.selected.has(entry.path) ? selectedPaths() : [entry.path];
    const payload: DragPayload = { session: session.id, paths };
    ev.dataTransfer.setData(DRAG_TYPE, JSON.stringify(payload));
    ev.dataTransfer.setData('text/plain', paths.join('\n'));
    ev.dataTransfer.effectAllowed = 'copy';
  }

  function ondropitems(ev: DragEvent, target: Entry | null) {
    if (!session) return;
    let payload: DragPayload;
    try {
      payload = JSON.parse(ev.dataTransfer?.getData(DRAG_TYPE) ?? '');
    } catch {
      return;
    }
    if (!payload?.paths?.length) return;
    const destDir = target?.path ?? path;
    // dropping onto the same folder of the same session is a no-op
    if (payload.session === session.id && payload.paths.every((p) => parentPath(p) === destDir)) return;
    if (target && payload.paths.includes(target.path)) return;
    onactivate();
    ontransfer({ fromSession: payload.session, paths: payload.paths, destDir });
  }

  function onkeyaction(action: 'delete' | 'rename' | 'properties' | 'copy' | 'preview' | 'edit') {
    if (action === 'preview') previewSelected();
    else if (action === 'edit') void editInEditor();
    else if (action === 'delete') deleteSelected();
    else if (action === 'rename') renameSelected();
    else if (action === 'properties') showProperties();
    else oncopy();
  }

  const total = $derived(entries.length);
  const statusText = $derived(
    selection.selected.size > 0
      ? t('itemsSelected', { count: selection.selected.size, total })
      : t('items', { total }),
  );
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section
  class="pane"
  class:active
  aria-label="{side === 'left' ? 'Left' : 'Right'} pane: {session?.label ?? ''}"
  bind:this={paneEl}
  onfocusin={onactivate}
  onmousedown={onactivate}
>
  <div class="top">
    <label class="sr-only" for="session-{side}">{t('session')}</label>
    <select
      id="session-{side}"
      class="session"
      value={sessionId ?? ''}
      onchange={(e) => onsessionchange((e.currentTarget as HTMLSelectElement).value)}
    >
      {#each sessions as s (s.id)}
        <option value={s.id}>{s.protocol === 'local' ? t('local') : `${s.label} (${s.protocol})`}</option>
      {/each}
    </select>
    <button type="button" class="btn icon ghost" title="{t('up')} (Backspace)" aria-label={t('up')} onclick={goUp} disabled={!parentPath(path)}>
      <Icon name="up" />
    </button>
    <Breadcrumb bind:this={breadcrumb} {path} onnavigate={(p) => navigate(p)} />
    <button type="button" class="btn icon ghost" title="{t('refresh')} (Ctrl+R)" aria-label={t('refresh')} onclick={refresh}>
      <Icon name="refresh" />
    </button>
  </div>
  <div class="tools">
    <div class="filter">
      <Icon name="search" size={14} />
      <input
        bind:this={filterInput}
        bind:value={filter}
        type="search"
        placeholder={t('filterPlaceholder')}
        aria-label={t('filter')}
        spellcheck="false"
        onkeydown={(e) => {
          if (e.key === 'Escape') {
            filter = '';
            focusList();
          } else if (e.key === 'ArrowDown' || e.key === 'Enter') {
            e.preventDefault();
            focusList();
          }
        }}
      />
    </div>
    <button
      type="button"
      class="btn icon ghost"
      aria-pressed={showHidden}
      aria-label={t('showHidden')}
      title="{t('showHidden')} (Ctrl+H)"
      onclick={toggleHidden}
    >
      <Icon name={showHidden ? 'eye' : 'eyeOff'} />
    </button>
    <button type="button" class="btn icon ghost" aria-label={t('newFolder')} title="{t('newFolder')} (F7)" onclick={newFolder}>
      <Icon name="plus" />
    </button>
  </div>
  {#if error}
    <div class="error" role="alert">{error}</div>
  {/if}
  <FileList
    {entries}
    {selection}
    {sort}
    {loading}
    label="{session?.label ?? ''} {path}"
    emptyText={raw.length === 0 ? t('emptyDir') : t('noMatches')}
    onselectionchange={(s) => (selection = s)}
    {onsort}
    onopen={open}
    onup={goUp}
    {oncontextmenu}
    {ondragstartrow}
    {ondropitems}
    {onkeyaction}
  />
  <div class="foot" aria-live="polite">
    <span>{loading ? t('loading') : statusText}</span>
  </div>
</section>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />
{/if}

{#if dialog?.kind === 'mkdir'}
  <InputDialog title={t('newFolderTitle')} label={t('name')} onsubmit={doMkdir} oncancel={() => (dialog = null)} />
{:else if dialog?.kind === 'rename'}
  {@const entry = dialog.entry}
  <InputDialog
    title={t('renameTitle')}
    label={t('name')}
    initial={entry.name}
    onsubmit={(n) => doRename(entry, n)}
    oncancel={() => (dialog = null)}
  />
{:else if dialog?.kind === 'delete'}
  {@const paths = dialog.paths}
  <ConfirmDialog
    title={t('deleteConfirmTitle')}
    message={t('deleteConfirm', {
      what: paths.length === 1 ? `"${baseName(paths[0])}"` : t('deleteMany', { count: paths.length }),
    })}
    confirmLabel={t('delete')}
    danger
    onconfirm={() => doDelete(paths)}
    oncancel={() => (dialog = null)}
  />
{:else if dialog?.kind === 'properties'}
  <PropertiesDialog entry={dialog.entry} onclose={() => (dialog = null)} />
{:else if dialog?.kind === 'preview' && session}
  <PreviewDialog
    session={session.id}
    entry={dialog.entry}
    oneditor={(e) => {
      dialog = null;
      void editInEditor(e);
    }}
    onclose={() => (dialog = null)}
  />
{/if}

<style>
  .pane {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }
  .pane.active {
    border-color: color-mix(in srgb, var(--accent) 55%, var(--border));
  }
  .top,
  .tools {
    display: flex;
    gap: 4px;
    align-items: center;
    padding: 6px 6px 0;
  }
  .tools {
    padding-bottom: 6px;
  }
  .session {
    max-width: 160px;
    flex: none;
  }
  .filter {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 6px;
    padding-left: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg);
    color: var(--fg-faint);
  }
  .filter input {
    flex: 1;
    border: none;
    background: transparent;
    min-width: 0;
    color: var(--fg);
    outline: none;
  }
  .filter:focus-within {
    outline: 2px solid var(--focus);
    outline-offset: 1px;
  }
  .error {
    margin: 0 6px 6px;
    padding: 6px 10px;
    border-radius: var(--radius-sm);
    background: var(--danger-soft);
    color: var(--danger);
    overflow-wrap: anywhere;
  }
  .foot {
    border-top: 1px solid var(--border);
    padding: 4px 10px;
    font-size: 12px;
    color: var(--fg-muted);
  }
</style>
