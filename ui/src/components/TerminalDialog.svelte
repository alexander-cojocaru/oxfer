<script lang="ts">
  import { onMount } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import '@xterm/xterm/css/xterm.css';
  import Modal from './Modal.svelte';
  import { api, errorMessage } from '../lib/api';
  import { t } from '../lib/i18n';
  import type { SessionInfo, TerminalEvent } from '../lib/types';

  let { session, onclose }: { session: SessionInfo; onclose: () => void } = $props();

  let host: HTMLDivElement | undefined = $state();
  let error = $state<string | null>(null);
  let ended = $state(false);

  const encoder = new TextEncoder();

  onMount(() => {
    if (!host) return;
    const term = new Terminal({
      cursorBlink: true,
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
      fontSize: 13,
      scrollback: 5000,
      theme: { background: '#14161c', foreground: '#e6e8ee' },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    fit.fit();
    term.focus();
    // Esc closes the window instead of going to the shell (xterm swallows it
    // before it can bubble to the dialog).
    term.attachCustomKeyEventHandler((e) => {
      if (e.key !== 'Escape') return true;
      if (e.type === 'keydown') onclose();
      return false;
    });

    let id: number | null = null;
    let disposed = false;
    // Events can arrive before `openTerminal` has returned the id.
    const early: TerminalEvent[] = [];

    const handle = (ev: TerminalEvent) => {
      if (ev.kind === 'output') {
        term.write(new Uint8Array(ev.data));
      } else {
        ended = true;
        term.write(`\r\n\x1b[2m[${t('terminalEnded')}]\x1b[0m\r\n`);
      }
    };
    const unlisten = api.onTerminalEvent((ev) => {
      if (id === null) early.push(ev);
      else if (ev.terminalId === id) handle(ev);
    });

    const send = (data: string) => {
      if (id === null || ended) return;
      api.terminalWrite(id, Array.from(encoder.encode(data))).catch((e) => (error = errorMessage(e)));
    };
    const input = term.onData(send);
    const binary = term.onBinary((s) => {
      if (id === null || ended) return;
      const bytes = Array.from(s, (c) => c.charCodeAt(0) & 0xff);
      api.terminalWrite(id, bytes).catch((e) => (error = errorMessage(e)));
    });

    const sync = () => {
      try {
        fit.fit();
      } catch {
        // host not laid out yet
      }
    };
    const resized = term.onResize(({ cols, rows }) => {
      if (id !== null && !ended) void api.terminalResize(id, cols, rows).catch(() => {});
    });
    const observer = new ResizeObserver(sync);
    observer.observe(host);

    api
      .openTerminal(session.id, term.cols, term.rows)
      .then((opened) => {
        if (disposed) {
          void api.closeTerminal(opened).catch(() => {});
          return;
        }
        id = opened;
        for (const ev of early.splice(0)) if (ev.terminalId === opened) handle(ev);
      })
      .catch((e) => {
        error = errorMessage(e);
      });

    return () => {
      disposed = true;
      observer.disconnect();
      input.dispose();
      binary.dispose();
      resized.dispose();
      void unlisten.then((f) => f());
      if (id !== null && !ended) void api.closeTerminal(id).catch(() => {});
      term.dispose();
    };
  });
</script>

<Modal title={t('terminalTitle', { name: session.label })} width={980}>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  <!-- Keys belong to the terminal: keep them from reaching dialog/app shortcuts. -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="term" bind:this={host} onkeydown={(e) => e.stopPropagation()}></div>
  {#snippet footer()}
    <button type="button" class="btn primary" onclick={onclose}>{t('close')}</button>
  {/snippet}
</Modal>

<style>
  .term {
    height: min(60vh, 540px);
    background: #14161c;
    border-radius: var(--radius-sm);
    padding: 6px 8px;
    overflow: hidden;
  }
  .error {
    margin: 0 0 8px;
    padding: 8px 10px;
    border-radius: var(--radius-sm);
    background: var(--danger-soft);
    color: var(--danger);
    overflow-wrap: anywhere;
  }
</style>
