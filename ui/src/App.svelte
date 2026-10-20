<script lang="ts">
  import { onMount } from 'svelte';
  import ChangelogDialog from './components/ChangelogDialog.svelte';
  import UpdateDialog from './components/UpdateDialog.svelte';
  import ConnectionDialog from './components/ConnectionDialog.svelte';
  import TransferConfigDialog from './components/TransferConfigDialog.svelte';
  import CredentialDialog from './components/CredentialDialog.svelte';
  import FilePane, { type TransferIntent } from './components/FilePane.svelte';
  import Icon from './components/Icon.svelte';
  import PromptDialog from './components/PromptDialog.svelte';
  import Sidebar from './components/Sidebar.svelte';
  import TerminalDialog from './components/TerminalDialog.svelte';
  import VncViewer from './components/VncViewer.svelte';
  import Toasts from './components/Toasts.svelte';
  import TransferQueue from './components/TransferQueue.svelte';
  import { api } from './lib/api';
  import { groupNames } from './lib/profiles';
  import { app } from './lib/state.svelte';
  import { formatSpeed } from './lib/format';
  import { parentPath } from './lib/files';
  import { t } from './lib/i18n';
  import { isActive, totalSpeed } from './lib/transfers';
  import { clamp, loadSize, QUEUE, saveSize, SIDEBAR, SPLIT } from './lib/layout';
  import Splitter from './components/Splitter.svelte';
  import {
    LOCAL_SESSION,
    type ConnectRequest,
    type Credential,
    type OverwritePolicy,
    type Profile,
    type SessionInfo,
  } from './lib/types';

  type Side = 'left' | 'right';

  let sessionIds = $state<Record<Side, string>>({ left: LOCAL_SESSION, right: LOCAL_SESSION });
  let activeSide = $state<Side>('left');
  let sidebarOpen = $state(true);
  let queueOpen = $state(true);
  let connecting = $state<string | null>(null);
  let dialog = $state<
    | { mode: 'quick' }
    | { mode: 'profile'; profile: Profile | null; duplicate?: boolean }
    | { mode: 'credential'; credential: Credential | null }
    | { mode: 'export' | 'import' }
    | null
  >(null);
  let changelogOpen = $state(false);
  let updateOpen = $state(false);
  let terminal = $state<SessionInfo | null>(null);
  let vnc = $state<{ request: ConnectRequest; label: string } | null>(null);
  let panes = $state<Record<Side, FilePane | undefined>>({ left: undefined, right: undefined });

  let sidebarW = $state(loadSize(SIDEBAR));
  let split = $state(loadSize(SPLIT));
  let queueH = $state(loadSize(QUEUE));
  let appEl = $state<HTMLElement>();
  let panesEl = $state<HTMLElement>();
  let queueEl = $state<HTMLElement>();

  function setSidebarW(v: number) {
    sidebarW = Math.round(clamp(v, SIDEBAR.min, SIDEBAR.max));
    saveSize(SIDEBAR.key, sidebarW);
  }
  function setSplit(v: number) {
    split = Math.round(clamp(v, SPLIT.min, SPLIT.max) * 1000) / 1000;
    saveSize(SPLIT.key, split);
  }
  function setQueueH(v: number) {
    queueH = Math.round(clamp(v, QUEUE.min, QUEUE.max));
    saveSize(QUEUE.key, queueH);
  }
  function dragSidebar(x: number) {
    if (appEl) setSidebarW(x - appEl.getBoundingClientRect().left - 8);
  }
  function dragSplit(x: number) {
    if (!panesEl) return;
    const r = panesEl.getBoundingClientRect();
    if (r.width > 0) setSplit((x - r.left) / r.width);
  }
  function dragQueue(y: number) {
    if (queueEl) setQueueH(queueEl.getBoundingClientRect().bottom - y);
  }

  const other = (s: Side): Side => (s === 'left' ? 'right' : 'left');
  const sessionFor = (side: Side): SessionInfo | null =>
    app.sessions.find((s) => s.id === sessionIds[side]) ?? null;
  const activeTransfers = $derived(app.queue.order.filter((id) => isActive(app.queue.byId[id])).length);
  const speed = $derived(totalSpeed(app.queue));
  const prompt = $derived(app.prompts[0]);

  onMount(() => {
    void app.loadSessions();
    void app.loadProfiles();
    const unlisten: Array<Promise<() => void>> = [
      api.onTransferEvent((ev) => {
        app.applyTransferEvent(ev);
        if (ev.type === 'done') {
          const tr = app.queue.byId[ev.transferId];
          for (const side of ['left', 'right'] as const) {
            if (tr && sessionIds[side] === tr.dstSession) panes[side]?.refresh();
          }
          if (!ev.cancelled) {
            app.toast(
              ev.failed ? 'error' : 'success',
              t('transferDone', { completed: ev.completed, skipped: ev.skipped, failed: ev.failed }),
            );
          }
        }
      }),
      api.onPrompt((ev) => {
        if (ev.type === 'open') app.openPrompt(ev.id, ev.prompt);
        else app.closePrompt(ev.id);
      }),
    ];
    app.status = t('ready');
    return () => unlisten.forEach((p) => p.then((f) => f()).catch(() => {}));
  });

  function onconnected(s: SessionInfo) {
    // The backend keeps one session per server and returns the open one.
    if (app.sessions.some((x) => x.id === s.id)) {
      const shown = (['left', 'right'] as const).find((side) => sessionIds[side] === s.id);
      const side: Side = shown ?? other(activeSide);
      sessionIds = { ...sessionIds, [side]: s.id };
      activeSide = side;
      panes[side]?.focusList();
      app.toast('info', t('alreadyConnected', { label: s.label }));
      return;
    }
    app.sessions = [...app.sessions, s];
    // show the new connection in the right pane (or the inactive one)
    const side: Side = sessionIds.right === LOCAL_SESSION ? 'right' : other(activeSide);
    sessionIds = { ...sessionIds, [side]: s.id };
    activeSide = side;
    app.toast('success', t('connected', { label: s.label }));
  }

  function openVnc(request: ConnectRequest, label: string) {
    vnc = { request, label };
  }

  async function connectProfile(p: Profile) {
    if (connecting) return;
    // A remote screen has no file session: the viewer connects itself.
    if (p.protocol === 'vnc') {
      openVnc({ kind: 'profile', name: p.name }, p.name);
      return;
    }
    connecting = p.name;
    try {
      onconnected(await api.connect({ kind: 'profile', name: p.name }));
    } catch (e) {
      app.error(e);
    } finally {
      connecting = null;
    }
  }

  // Connects first (or reuses the open session for that server), then opens a shell.
  async function profileTerminal(p: Profile) {
    if (connecting) return;
    connecting = p.name;
    try {
      const s = await api.connect({ kind: 'profile', name: p.name });
      onconnected(s);
      terminal = s;
    } catch (e) {
      app.error(e);
    } finally {
      connecting = null;
    }
  }

  async function disconnect(s: SessionInfo) {
    try {
      await api.disconnect(s.id);
    } catch (e) {
      app.error(e);
    }
    app.sessions = app.sessions.filter((x) => x.id !== s.id);
    if (terminal?.id === s.id) terminal = null;
    for (const side of ['left', 'right'] as const) {
      if (sessionIds[side] === s.id) sessionIds = { ...sessionIds, [side]: LOCAL_SESSION };
    }
    app.toast('info', t('disconnected', { label: s.label }));
  }

  async function transfer(toSide: Side, intent: TransferIntent) {
    const dst = sessionIds[toSide];
    if (intent.paths.length === 0) {
      app.toast('info', t('noSelection'));
      return;
    }
    if (intent.fromSession === dst && intent.paths.every((p) => parentPath(p) === intent.destDir)) {
      app.toast('info', t('sameSession'));
      return;
    }
    try {
      await api.startTransfer({
        srcSession: intent.fromSession,
        dstSession: dst,
        sources: intent.paths,
        destDir: intent.destDir,
        options: { overwrite: app.overwrite },
      });
      queueOpen = true;
    } catch (e) {
      app.error(e);
    }
  }

  function copyFrom(side: Side) {
    const src = panes[side];
    const dst = panes[other(side)];
    if (!src || !dst) return;
    void transfer(other(side), {
      fromSession: sessionIds[side],
      paths: src.selectedPaths(),
      destDir: dst.currentPath(),
    });
  }

  function modalOpen() {
    return document.querySelector('[aria-modal="true"], [role="menu"]') !== null;
  }

  function onkeydown(e: KeyboardEvent) {
    if (modalOpen()) return;
    const mod = e.ctrlKey || e.metaKey;
    const pane = panes[activeSide];
    const key = e.key.toLowerCase();
    if (e.key === 'F5') {
      copyFrom(activeSide);
    } else if (e.key === 'F7') {
      pane?.newFolder();
    } else if (mod && key === 'r') {
      pane?.refresh();
    } else if (mod && key === 'l') {
      pane?.editPath();
    } else if (mod && key === 'f') {
      pane?.focusFilter();
    } else if (mod && key === 'h') {
      pane?.toggleHidden();
    } else if (mod && key === 'n') {
      dialog = { mode: 'quick' };
    } else if (mod && key === 'b') {
      sidebarOpen = !sidebarOpen;
    } else if (mod && key === 'j') {
      queueOpen = !queueOpen;
    } else if (mod && (key === '1' || key === '2')) {
      activeSide = key === '1' ? 'left' : 'right';
      panes[activeSide]?.focusList();
    } else {
      return;
    }
    e.preventDefault();
  }

  const policies: { value: OverwritePolicy; label: string }[] = [
    { value: 'ask', label: t('overwriteAsk') },
    { value: 'overwrite', label: t('overwriteAlways') },
    { value: 'newerOnly', label: t('overwriteNewer') },
    { value: 'skip', label: t('overwriteSkip') },
  ];
</script>

<svelte:window {onkeydown} />

<div
  class="app"
  class:no-sidebar={!sidebarOpen}
  class:no-queue={!queueOpen}
  bind:this={appEl}
  style:--sidebar-w="{sidebarW}px"
  style:--queue-h="{queueH}px"
>
  <header class="toolbar">
    <button
      type="button"
      class="btn icon ghost"
      aria-label={t('sidebar')}
      aria-pressed={sidebarOpen}
      title="{t('sidebar')} (Ctrl+B)"
      onclick={() => (sidebarOpen = !sidebarOpen)}
    >
      <Icon name="sidebar" />
    </button>
    <span class="brand"><span class="logo" aria-hidden="true"></span>oxfer</span>
    <div class="spacer"></div>
    <button type="button" class="btn primary" onclick={() => (dialog = { mode: 'quick' })} title="{t('quickConnect')} (Ctrl+N)">
      <Icon name="plug" />{t('quickConnect')}
    </button>
    <button
      type="button"
      class="btn"
      onclick={() => copyFrom(activeSide)}
      title="{t('copyToOther')} (F5)"
      aria-label={t('copyToOther')}
    >
      <Icon name="copy" /><span class="hide-narrow">{t('copyToOther')}</span>
    </button>
    <label class="policy">
      <span class="hide-narrow">{t('whenExists')}</span>
      <select bind:value={app.overwrite} aria-label={t('whenExists')}>
        {#each policies as p (p.value)}
          <option value={p.value}>{p.label}</option>
        {/each}
      </select>
    </label>
  </header>

  {#if sidebarOpen}
    <Sidebar
      {connecting}
      onconnectprofile={connectProfile}
      onnewprofile={() => (dialog = { mode: 'profile', profile: null })}
      oneditprofile={(p) => (dialog = { mode: 'profile', profile: p })}
      onduplicateprofile={(p) => (dialog = { mode: 'profile', profile: p, duplicate: true })}
      onconfigtransfer={(mode) => (dialog = { mode })}
      onnewcredential={() => (dialog = { mode: 'credential', credential: null })}
      oneditcredential={(c) => (dialog = { mode: 'credential', credential: c })}
      ondisconnect={disconnect}
      onterminal={(s) => (terminal = s)}
      onprofileterminal={profileTerminal}
    />
    <div class="vsplit">
      <Splitter
        orientation="vertical"
        label={t('resizeSidebar')}
        onmove={dragSidebar}
        onstep={(d, big) => setSidebarW(sidebarW + d * (big ? 40 : 10))}
        onreset={() => setSidebarW(SIDEBAR.def)}
      />
    </div>
  {/if}

  <main class="panes" bind:this={panesEl} style:grid-template-columns="minmax(0, {split}fr) 8px minmax(0, {1 - split}fr)">
    {#each ['left', 'right'] as const as side (side)}
      {#if side === 'right'}
        <Splitter
          orientation="vertical"
          label={t('resizePanes')}
          onmove={dragSplit}
          onstep={(d, big) => setSplit(split + d * (big ? 0.1 : 0.02))}
          onreset={() => setSplit(SPLIT.def)}
        />
      {/if}
      <FilePane
        bind:this={panes[side]}
        {side}
        session={sessionFor(side)}
        sessions={app.sessions}
        active={activeSide === side}
        onactivate={() => (activeSide = side)}
        onsessionchange={(id) => (sessionIds = { ...sessionIds, [side]: id })}
        ontransfer={(intent) => transfer(side, intent)}
        oncopy={() => copyFrom(side)}
      />
    {/each}
  </main>

  {#if queueOpen}
    <div class="hsplit">
      <Splitter
        orientation="horizontal"
        label={t('resizeQueue')}
        onmove={dragQueue}
        onstep={(d, big) => setQueueH(queueH - d * (big ? 60 : 20))}
        onreset={() => setQueueH(QUEUE.def)}
      />
    </div>
    <section class="queue" aria-label={t('transfers')} bind:this={queueEl}>
      <div class="queue-head">
        <h2>{t('transfers')}</h2>
        <div class="spacer"></div>
        <button type="button" class="btn ghost" onclick={() => app.clearFinished()}>{t('clearFinished')}</button>
        <button type="button" class="btn icon ghost" aria-label={t('close')} title="{t('close')} (Ctrl+J)" onclick={() => (queueOpen = false)}>
          <Icon name="x" />
        </button>
      </div>
      <TransferQueue queue={app.queue} />
    </section>
  {/if}

  <footer class="statusbar">
    <span class="status" aria-live="polite">{app.status}</span>
    <div class="spacer"></div>
    <button type="button" class="link" title={t('changelog')} onclick={() => (changelogOpen = true)}>
      v{__APP_VERSION__}
    </button>
    <button type="button" class="link" onclick={() => (updateOpen = true)}>{t('checkForUpdates')}</button>
    <button type="button" class="link" onclick={() => (queueOpen = !queueOpen)} aria-pressed={queueOpen}>
      {t('transfers')}: {t('activeTransfers', { count: activeTransfers })}
      {#if activeTransfers}· {formatSpeed(speed)}{/if}
    </button>
  </footer>
</div>

{#if dialog?.mode === 'quick'}
  <ConnectionDialog mode="quick" groups={groupNames(app.profiles)} credentials={app.credentials} {onconnected} onvnc={openVnc} onsaved={() => app.loadProfiles()} onclose={() => (dialog = null)} />
{:else if dialog?.mode === 'profile'}
  <ConnectionDialog
    mode="profile"
    profile={dialog.profile}
    duplicate={dialog.duplicate}
    groups={groupNames(app.profiles)}
    credentials={app.credentials}
    onvnc={openVnc}
    onsaved={() => app.loadProfiles()}
    onclose={() => (dialog = null)}
  />
{:else if dialog?.mode === 'export' || dialog?.mode === 'import'}
  <TransferConfigDialog mode={dialog.mode} onclose={() => (dialog = null)} />
{:else if dialog?.mode === 'credential'}
  <CredentialDialog credential={dialog.credential} onsaved={() => app.loadProfiles()} onclose={() => (dialog = null)} />
{/if}

{#if changelogOpen}
  <ChangelogDialog onclose={() => (changelogOpen = false)} />
{/if}

{#if updateOpen}
  <UpdateDialog {activeTransfers} onclose={() => (updateOpen = false)} />
{/if}

{#if terminal}
  <TerminalDialog session={terminal} onclose={() => (terminal = null)} />
{/if}

{#if vnc}
  <VncViewer request={vnc.request} label={vnc.label} onclose={() => (vnc = null)} />
{/if}

{#if prompt}
  {#key prompt.id}
    <PromptDialog id={prompt.id} prompt={prompt.prompt} onanswer={(id, a) => app.answerPrompt(id, a)} />
  {/key}
{/if}

<Toasts />

<style>
  .app {
    height: 100%;
    display: grid;
    grid-template-columns: var(--sidebar-w, 240px) 8px minmax(0, 1fr);
    grid-template-rows: auto minmax(0, 1fr) 8px clamp(80px, var(--queue-h, 200px), 60%) auto;
    grid-template-areas:
      'toolbar toolbar toolbar'
      'sidebar vsplit panes'
      'sidebar vsplit hsplit'
      'sidebar vsplit queue'
      'status status status';
    padding: 0 8px;
  }
  .app.no-sidebar {
    grid-template-columns: minmax(0, 1fr);
    grid-template-areas:
      'toolbar'
      'panes'
      'hsplit'
      'queue'
      'status';
  }
  .app.no-queue {
    grid-template-rows: auto minmax(0, 1fr) 0 0 auto;
  }
  .app.no-queue:not(.no-sidebar) {
    grid-template-areas:
      'toolbar toolbar toolbar'
      'sidebar vsplit panes'
      'sidebar vsplit panes'
      'sidebar vsplit panes'
      'status status status';
  }
  .app.no-queue.no-sidebar {
    grid-template-areas:
      'toolbar'
      'panes'
      'panes'
      'panes'
      'status';
  }
  .vsplit {
    grid-area: vsplit;
    display: grid;
  }
  .hsplit {
    grid-area: hsplit;
    display: grid;
  }
  .toolbar {
    grid-area: toolbar;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 0;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 700;
    font-size: 15px;
    letter-spacing: 0.02em;
  }
  .logo {
    width: 18px;
    height: 18px;
    border-radius: 5px;
    background: linear-gradient(135deg, var(--accent), var(--accent-2));
  }
  .spacer {
    flex: 1;
  }
  .policy {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--fg-muted);
  }
  .app > :global(aside) {
    grid-area: sidebar;
  }
  .panes {
    grid-area: panes;
    display: grid;
    min-height: 0;
  }
  .queue {
    grid-area: queue;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }
  .queue-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px 4px 12px;
    border-bottom: 1px solid var(--border);
  }
  .queue-head h2 {
    font-size: 12px;
    margin: 0;
    font-weight: 600;
  }
  .statusbar {
    grid-area: status;
    display: flex;
    align-items: center;
    gap: 12px;
    font-size: 12px;
    color: var(--fg-muted);
    padding: 8px 0 6px;
  }
  .status {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .link {
    background: none;
    border: none;
    color: inherit;
    padding: 2px 6px;
    border-radius: 4px;
    font-variant-numeric: tabular-nums;
  }
  .link:hover {
    color: var(--fg);
    background: var(--bg-hover);
  }
  @media (max-width: 1000px) {
    .hide-narrow {
      display: none;
    }
  }
</style>
