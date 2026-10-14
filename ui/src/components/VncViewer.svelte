<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from './Icon.svelte';
  import { api, errorMessage } from '../lib/api';
  import { app } from '../lib/state.svelte';
  import { t } from '../lib/i18n';
  import {
    buttonMask,
    CTRL_ALT_DEL,
    keysymFor,
    parseVncMessage,
    screenPoint,
    viewScale,
    wheelMask,
    type VncMessage,
  } from '../lib/vnc';
  import type { ConnectRequest } from '../lib/types';

  let { request, label, onclose }: { request: ConnectRequest; label: string; onclose: () => void } = $props();

  type Phase = 'connecting' | 'live' | 'ended';

  let canvas: HTMLCanvasElement | undefined = $state();
  let stage: HTMLDivElement | undefined = $state();
  let reconnectButton: HTMLButtonElement | undefined = $state();
  let phase = $state<Phase>('connecting');
  let error = $state<string | null>(null);
  let screen = $state({ width: 0, height: 0 });
  let area = $state({ width: 0, height: 0 });
  let mode = $state<'fit' | 'actual'>('fit');
  const scale = $derived(viewScale(mode, screen, area));

  let id: number | null = null;
  let generation = 0;
  let disposed = false;
  // Messages that arrive before `vncOpen` has returned the screen size.
  let early: ArrayBuffer[] | null = null;
  // Clipboard text last exchanged with the remote side (avoids echoing it back).
  let lastClipboard = '';

  // --- drawing -------------------------------------------------------------

  function resizeScreen(width: number, height: number) {
    if (!canvas) return;
    canvas.width = width;
    canvas.height = height;
    screen = { width, height };
  }

  function apply(msg: VncMessage) {
    switch (msg.kind) {
      case 'update': {
        const ctx = canvas?.getContext('2d');
        if (!canvas || !ctx) return;
        for (const op of msg.ops) {
          if (op.type === 'resize') resizeScreen(op.width, op.height);
          else if (op.type === 'rect') ctx.putImageData(new ImageData(op.rgba, op.width, op.height), op.x, op.y);
          else ctx.drawImage(canvas, op.srcX, op.srcY, op.width, op.height, op.x, op.y, op.width, op.height);
        }
        break;
      }
      case 'clipboard':
        lastClipboard = msg.text;
        void navigator.clipboard?.writeText(msg.text).catch(() => {});
        break;
      case 'bell':
        break;
      case 'closed':
        id = null;
        releaseKeys();
        phase = 'ended';
        error = msg.reason;
        break;
    }
  }

  function receive(data: ArrayBuffer) {
    try {
      apply(parseVncMessage(data));
    } catch (e) {
      console.error('bad screen message', e);
    }
  }

  async function open() {
    const gen = ++generation;
    phase = 'connecting';
    error = null;
    early = [];
    try {
      const info = await api.vncOpen(request, (data) => {
        if (gen !== generation) return;
        if (early) early.push(data);
        else receive(data);
      });
      if (disposed || gen !== generation) {
        void api.vncClose(info.id).catch(() => {});
        return;
      }
      id = info.id;
      resizeScreen(info.width, info.height);
      phase = 'live';
      const queued = early ?? [];
      early = null;
      queued.forEach(receive);
      canvas?.focus();
    } catch (e) {
      if (gen !== generation) return;
      early = null;
      phase = 'ended';
      error = errorMessage(e);
    }
  }

  // --- keyboard ------------------------------------------------------------

  // Key events are sent in order: a paste first sends the clipboard.
  let chain: Promise<unknown> = Promise.resolve();
  function enqueue(job: () => Promise<unknown> | unknown) {
    chain = chain.then(job).catch(() => {});
  }

  function sendKey(keysym: number, down: boolean) {
    if (id === null) return;
    const target = id;
    enqueue(() => api.vncKey(target, keysym, down));
  }

  // `code` -> keysym of the keys currently held, so the release matches the press.
  const pressed = new Map<string, number>();

  function releaseKeys(except?: string) {
    for (const [code, sym] of pressed) {
      if (code === except) continue;
      sendKey(sym, false);
      pressed.delete(code);
    }
  }

  const sleep = (ms: number) => new Promise<null>((r) => setTimeout(() => r(null), ms));

  /** Send the local clipboard before a paste shortcut reaches the remote side. */
  async function sendLocalClipboard() {
    if (id === null) return;
    try {
      const text = await Promise.race([navigator.clipboard.readText(), sleep(500)]);
      if (text && text !== lastClipboard) {
        lastClipboard = text;
        await api.vncClipboard(id, text);
      }
    } catch {
      // no permission: the paste goes through with the remote clipboard
    }
  }

  function onkeydown(e: KeyboardEvent) {
    // Everything, even Tab and Escape, belongs to the remote screen.
    e.preventDefault();
    e.stopPropagation();
    if (phase !== 'live' || e.isComposing) return;
    const sym = keysymFor(e);
    if (sym === null) return;
    const held = pressed.get(e.code);
    if (held !== undefined && held !== sym) sendKey(held, false);
    pressed.set(e.code, sym);
    if (!e.repeat && !e.altKey && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'v') {
      enqueue(sendLocalClipboard);
    }
    sendKey(sym, true);
  }

  function onkeyup(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (phase !== 'live') return;
    const sym = pressed.get(e.code) ?? keysymFor(e);
    pressed.delete(e.code);
    if (sym !== null) sendKey(sym, false);
    // macOS sends no key-up for other keys while Command is held.
    if (e.code === 'MetaLeft' || e.code === 'MetaRight') releaseKeys();
  }

  // --- pointer -------------------------------------------------------------

  let lastButtons = 0;
  let queuedMove: { x: number; y: number; buttons: number } | null = null;
  let frame = 0;
  let wheel = { x: 0, y: 0 };

  function pointerAt(e: MouseEvent) {
    if (!canvas) return { x: 0, y: 0 };
    return screenPoint(e.clientX, e.clientY, canvas.getBoundingClientRect(), screen);
  }

  function sendPointer(x: number, y: number, buttons: number) {
    if (id !== null) void api.vncPointer(id, x, y, buttons).catch(() => {});
  }

  function flushMove() {
    frame = 0;
    if (queuedMove) sendPointer(queuedMove.x, queuedMove.y, queuedMove.buttons);
    queuedMove = null;
  }

  function onpointer(e: PointerEvent) {
    if (phase !== 'live') return;
    const buttons = buttonMask(e.buttons);
    const p = pointerAt(e);
    if (buttons !== lastButtons) {
      // Button changes are never coalesced.
      queuedMove = null;
      lastButtons = buttons;
      sendPointer(p.x, p.y, buttons);
      return;
    }
    // Moves are sent at most once per frame.
    queuedMove = { ...p, buttons };
    if (!frame) frame = requestAnimationFrame(flushMove);
  }

  function onpointerdown(e: PointerEvent) {
    canvas?.focus();
    canvas?.setPointerCapture(e.pointerId);
    onpointer(e);
  }

  const WHEEL_STEP = 40;

  function onwheel(e: WheelEvent) {
    e.preventDefault();
    if (phase !== 'live') return;
    const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1;
    wheel.x += e.deltaX * unit;
    wheel.y += e.deltaY * unit;
    const p = pointerAt(e);
    // One wheel "button" click per step of accumulated scrolling.
    while (Math.abs(wheel.y) >= WHEEL_STEP || Math.abs(wheel.x) >= WHEEL_STEP) {
      const vertical = Math.abs(wheel.y) >= WHEEL_STEP;
      const mask = vertical ? wheelMask(0, wheel.y) : wheelMask(wheel.x, 0);
      if (vertical) wheel.y -= Math.sign(wheel.y) * WHEEL_STEP;
      else wheel.x -= Math.sign(wheel.x) * WHEEL_STEP;
      sendPointer(p.x, p.y, lastButtons | mask);
      sendPointer(p.x, p.y, lastButtons);
    }
  }

  // --- toolbar -------------------------------------------------------------

  function ctrlAltDel() {
    for (const sym of CTRL_ALT_DEL) sendKey(sym, true);
    for (const sym of [...CTRL_ALT_DEL].reverse()) sendKey(sym, false);
    canvas?.focus();
  }

  async function sendClipboard() {
    if (id === null) return;
    try {
      const text = await navigator.clipboard.readText();
      lastClipboard = text;
      await api.vncClipboard(id, text);
      app.toast('info', t('vncClipboardSent'));
    } catch (e) {
      app.toast('error', e instanceof Error || typeof e === 'string' ? t('vncClipboardFailed') : errorMessage(e));
    }
    canvas?.focus();
  }

  $effect(() => {
    if (phase === 'ended') reconnectButton?.focus();
  });

  function reconnect() {
    void open();
  }

  onMount(() => {
    const observer = new ResizeObserver(() => {
      if (stage) area = { width: stage.clientWidth, height: stage.clientHeight };
    });
    if (stage) observer.observe(stage);
    void open();
    return () => {
      disposed = true;
      generation++;
      observer.disconnect();
      if (frame) cancelAnimationFrame(frame);
      if (id !== null) void api.vncClose(id).catch(() => {});
    };
  });
</script>

<div class="viewer" role="dialog" aria-modal="true" aria-label={t('vncTitle', { name: label })}>
  <header>
    <Icon name="monitor" size={15} />
    <h2>{t('vncTitle', { name: label })}</h2>
    <span class="state" class:live={phase === 'live'}>
      {phase === 'live' ? `${screen.width}×${screen.height}` : phase === 'connecting' ? t('connecting') : t('vncDisconnected')}
    </span>
    <span class="spacer"></span>
    <button
      type="button"
      class="btn"
      onclick={() => (mode = mode === 'fit' ? 'actual' : 'fit')}
      aria-pressed={mode === 'fit'}
      title={mode === 'fit' ? t('vncActualSize') : t('vncFit')}
    >
      {mode === 'fit' ? t('vncActualSize') : t('vncFit')}
    </button>
    <button type="button" class="btn" onclick={ctrlAltDel} disabled={phase !== 'live'}>{t('vncCtrlAltDel')}</button>
    <button type="button" class="btn" onclick={sendClipboard} disabled={phase !== 'live'}>{t('vncSendClipboard')}</button>
    <button type="button" class="btn primary" onclick={onclose}>{t('disconnect')}</button>
  </header>

  <div class="stage" bind:this={stage} class:scroll={mode === 'actual'}>
    <!-- The canvas is the keyboard and mouse input of the remote screen. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <canvas
      bind:this={canvas}
      class:dim={phase !== 'live'}
      aria-label={t('vncCanvasLabel')}
      tabindex="0"
      style:width="{screen.width * scale}px"
      style:height="{screen.height * scale}px"
      {onkeydown}
      {onkeyup}
      onblur={() => releaseKeys()}
      onpointermove={onpointer}
      onpointerdown={onpointerdown}
      onpointerup={onpointer}
      onpointercancel={onpointer}
      {onwheel}
      oncontextmenu={(e) => e.preventDefault()}
    ></canvas>

    {#if phase === 'connecting'}
      <div class="overlay" role="status">
        <span class="spinner" aria-hidden="true"></span>
        <p>{t('vncConnecting', { name: label })}</p>
      </div>
    {:else if phase === 'ended'}
      <div class="overlay" role="alert">
        <p class="reason">{error ?? t('vncDisconnected')}</p>
        <div class="actions">
          <button type="button" class="btn primary" onclick={reconnect} bind:this={reconnectButton}>{t('vncReconnect')}</button>
          <button type="button" class="btn" onclick={onclose}>{t('close')}</button>
        </div>
      </div>
    {/if}
  </div>
  <footer>{t('vncKeysHint')}</footer>
</div>

<style>
  .viewer {
    position: fixed;
    inset: 0;
    /* Below dialogs (100), so a password question shows over the screen. */
    z-index: 50;
    display: flex;
    flex-direction: column;
    background: #0b0c10;
    color: #e6e8ee;
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: var(--bg-elev);
    color: var(--fg);
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .state {
    font-size: 12px;
    color: var(--fg-muted);
  }
  .state.live {
    color: var(--ok);
  }
  .spacer {
    flex: 1;
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    overflow: hidden;
  }
  .stage.scroll {
    overflow: auto;
  }
  canvas {
    margin: auto;
    flex: none;
    outline: none;
    background: #000;
    touch-action: none;
  }
  canvas:focus-visible {
    box-shadow: 0 0 0 2px var(--accent);
  }
  canvas.dim {
    opacity: 0.35;
  }
  .overlay {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    text-align: center;
    padding: 16px;
  }
  .overlay p {
    margin: 0;
    max-width: 520px;
    overflow-wrap: anywhere;
  }
  .reason {
    padding: 8px 12px;
    border-radius: var(--radius-sm);
    background: var(--danger-soft);
    color: var(--danger);
  }
  .actions {
    display: flex;
    gap: 8px;
  }
  .spinner {
    width: 22px;
    height: 22px;
    border: 3px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  footer {
    padding: 4px 12px;
    font-size: 11px;
    color: var(--fg-muted);
    background: var(--bg-elev);
    border-top: 1px solid var(--border);
  }
</style>
