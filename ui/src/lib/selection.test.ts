import { describe, expect, it } from 'vitest';
import { click, emptySelection, moveFocus, prune, selectAll, selectedInOrder, toggleFocused } from './selection';

const items = ['a', 'b', 'c', 'd', 'e'];
const sel = (s: ReturnType<typeof click>) => [...s.selected].sort();

describe('selection', () => {
  it('plain click selects one item', () => {
    let s = click(emptySelection, items, 'b');
    expect(sel(s)).toEqual(['b']);
    s = click(s, items, 'd');
    expect(sel(s)).toEqual(['d']);
    expect(s.anchor).toBe('d');
  });

  it('shift-click selects a range from the anchor (both directions)', () => {
    let s = click(emptySelection, items, 'b');
    s = click(s, items, 'd', { shift: true });
    expect(sel(s)).toEqual(['b', 'c', 'd']);
    s = click(s, items, 'a', { shift: true });
    expect(sel(s)).toEqual(['a', 'b']);
    expect(s.anchor).toBe('b');
  });

  it('ctrl-click toggles and moves the anchor', () => {
    let s = click(emptySelection, items, 'a');
    s = click(s, items, 'c', { toggle: true });
    expect(sel(s)).toEqual(['a', 'c']);
    s = click(s, items, 'a', { toggle: true });
    expect(sel(s)).toEqual(['c']);
    // ctrl+shift adds the range from the anchor (last ctrl-clicked) to the selection
    s = click(s, items, 'd', { toggle: true });
    s = click(s, items, 'e', { shift: true, toggle: true });
    expect(sel(s)).toEqual(['c', 'd', 'e']);
  });

  it('keyboard movement and shift extension', () => {
    let s = moveFocus(emptySelection, items, 1);
    expect(s.focus).toBe('a');
    s = moveFocus(s, items, 1);
    expect(sel(s)).toEqual(['b']);
    s = moveFocus(s, items, 2, { shift: true });
    expect(sel(s)).toEqual(['b', 'c', 'd']);
    s = moveFocus(s, items, 'home', { shift: true });
    expect(sel(s)).toEqual(['a', 'b']);
    s = moveFocus(s, items, 'end');
    expect(sel(s)).toEqual(['e']);
    s = moveFocus(s, items, 10);
    expect(s.focus).toBe('e');
    // ctrl+arrow moves only the cursor, space toggles
    s = moveFocus(s, items, -2, { toggle: true });
    expect(s.focus).toBe('c');
    expect(sel(s)).toEqual(['e']);
    s = toggleFocused(s);
    expect(sel(s)).toEqual(['c', 'e']);
  });

  it('select all, prune and ordering', () => {
    let s = selectAll(items, emptySelection);
    expect(s.selected.size).toBe(5);
    s = prune(s, ['b', 'e']);
    expect(sel(s)).toEqual(['b', 'e']);
    expect(selectedInOrder(click(click(emptySelection, items, 'e'), items, 'a', { toggle: true }), items)).toEqual([
      'a',
      'e',
    ]);
    expect(moveFocus(s, [], 1)).toEqual(emptySelection);
  });
});
