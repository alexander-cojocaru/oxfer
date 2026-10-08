// Pure multi-selection logic for file lists (click / shift / ctrl / keyboard).
// `items` is the ordered list of visible keys (entry paths).

export interface Selection {
  selected: ReadonlySet<string>;
  /** Fixed end of a shift-range. */
  anchor: string | null;
  /** Keyboard cursor. */
  focus: string | null;
}

export interface Modifiers {
  shift?: boolean;
  /** Ctrl on Windows/Linux, Cmd on macOS. */
  toggle?: boolean;
}

export const emptySelection: Selection = { selected: new Set(), anchor: null, focus: null };

function range(items: readonly string[], a: string, b: string): string[] {
  const i = items.indexOf(a);
  const j = items.indexOf(b);
  if (i < 0 || j < 0) return [b];
  const [lo, hi] = i <= j ? [i, j] : [j, i];
  return items.slice(lo, hi + 1);
}

export function click(sel: Selection, items: readonly string[], key: string, mods: Modifiers = {}): Selection {
  if (mods.shift && sel.anchor && items.includes(sel.anchor)) {
    const r = range(items, sel.anchor, key);
    const selected = mods.toggle ? new Set([...sel.selected, ...r]) : new Set(r);
    return { selected, anchor: sel.anchor, focus: key };
  }
  if (mods.toggle) {
    const selected = new Set(sel.selected);
    if (selected.has(key)) selected.delete(key);
    else selected.add(key);
    return { selected, anchor: key, focus: key };
  }
  return { selected: new Set([key]), anchor: key, focus: key };
}

export type Move = number | 'home' | 'end';

/** Move the cursor by `move` rows; shift extends the range from the anchor. */
export function moveFocus(
  sel: Selection,
  items: readonly string[],
  move: Move,
  mods: Modifiers = {},
): Selection {
  if (items.length === 0) return emptySelection;
  const cur = sel.focus ? items.indexOf(sel.focus) : -1;
  let next: number;
  if (move === 'home') next = 0;
  else if (move === 'end') next = items.length - 1;
  else if (cur < 0) next = move > 0 ? 0 : items.length - 1;
  else next = Math.min(items.length - 1, Math.max(0, cur + move));
  const key = items[next];
  if (mods.shift) {
    const anchor = sel.anchor && items.includes(sel.anchor) ? sel.anchor : (sel.focus ?? key);
    return { selected: new Set(range(items, anchor, key)), anchor, focus: key };
  }
  if (mods.toggle) {
    // move the cursor only (Ctrl+arrows), keep the selection
    return { ...sel, focus: key };
  }
  return { selected: new Set([key]), anchor: key, focus: key };
}

/** Space: toggle the focused item. */
export function toggleFocused(sel: Selection): Selection {
  if (!sel.focus) return sel;
  const selected = new Set(sel.selected);
  if (selected.has(sel.focus)) selected.delete(sel.focus);
  else selected.add(sel.focus);
  return { selected, anchor: sel.focus, focus: sel.focus };
}

export function selectAll(items: readonly string[], sel: Selection): Selection {
  return { selected: new Set(items), anchor: sel.anchor ?? items[0] ?? null, focus: sel.focus ?? items[0] ?? null };
}

/** Drop keys that are no longer visible (after refresh / filter). */
export function prune(sel: Selection, items: readonly string[]): Selection {
  const visible = new Set(items);
  const selected = new Set([...sel.selected].filter((k) => visible.has(k)));
  return {
    selected,
    anchor: sel.anchor && visible.has(sel.anchor) ? sel.anchor : null,
    focus: sel.focus && visible.has(sel.focus) ? sel.focus : null,
  };
}

/** Selected keys in display order. */
export function selectedInOrder(sel: Selection, items: readonly string[]): string[] {
  return items.filter((k) => sel.selected.has(k));
}
