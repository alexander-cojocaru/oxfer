<script lang="ts">
  interface Props {
    orientation: 'vertical' | 'horizontal';
    label: string;
    /** Called with the pointer position (clientX for vertical, clientY for horizontal) while dragging. */
    onmove: (pos: number) => void;
    /** Keyboard nudge: -1 / +1 in the direction of increasing x/y; `big` for shift. */
    onstep: (dir: -1 | 1, big: boolean) => void;
    onreset: () => void;
  }
  let { orientation, label, onmove, onstep, onreset }: Props = $props();

  let dragging = $state(false);

  function pos(e: PointerEvent) {
    return orientation === 'vertical' ? e.clientX : e.clientY;
  }

  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    dragging = true;
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    e.preventDefault();
  }

  function move(e: PointerEvent) {
    if (dragging) onmove(pos(e));
  }

  function up() {
    dragging = false;
  }

  function key(e: KeyboardEvent) {
    const prev = orientation === 'vertical' ? 'ArrowLeft' : 'ArrowUp';
    const next = orientation === 'vertical' ? 'ArrowRight' : 'ArrowDown';
    if (e.key === prev) onstep(-1, e.shiftKey);
    else if (e.key === next) onstep(1, e.shiftKey);
    else if (e.key === 'Enter') onreset();
    else return;
    e.preventDefault();
  }
</script>

<!-- A focusable separator is a widget (WAI-ARIA window splitter); svelte's a11y lint doesn't know that. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class="splitter {orientation}"
  class:dragging
  role="separator"
  tabindex="0"
  aria-label={label}
  aria-orientation={orientation}
  title={label}
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={up}
  ondblclick={onreset}
  onkeydown={key}
></div>

<style>
  .splitter {
    position: relative;
    touch-action: none;
    border-radius: 3px;
    outline: none;
  }
  .splitter.vertical {
    cursor: col-resize;
  }
  .splitter.horizontal {
    cursor: row-resize;
  }
  .splitter::after {
    content: '';
    position: absolute;
    inset: 0;
    margin: auto;
    border-radius: 2px;
    background: transparent;
    transition: background 0.12s;
  }
  .splitter.vertical::after {
    width: 3px;
    height: 100%;
  }
  .splitter.horizontal::after {
    height: 3px;
    width: 100%;
  }
  .splitter:hover::after,
  .splitter:focus-visible::after,
  .splitter.dragging::after {
    background: var(--accent);
  }
  .splitter.dragging {
    user-select: none;
  }
</style>
