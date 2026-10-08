import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import FileList from './FileList.svelte';
import { emptySelection, type Selection } from '../lib/selection';
import type { Entry } from '../lib/types';

function entries(n: number): Entry[] {
  return Array.from({ length: n }, (_, i) => ({
    name: `file${i}.txt`,
    path: `/d/file${i}.txt`,
    kind: i === 0 ? ('dir' as const) : ('file' as const),
    size: i * 10,
    modified: 0,
    permissions: null,
  }));
}

function setup(n = 5, selection: Selection = emptySelection) {
  const props = {
    entries: entries(n),
    selection,
    sort: { key: 'name' as const, dir: 'asc' as const },
    label: 'test list',
    emptyText: 'empty',
    onselectionchange: vi.fn(),
    onsort: vi.fn(),
    onopen: vi.fn(),
    onup: vi.fn(),
    onkeyaction: vi.fn(),
  };
  const r = render(FileList, { props });
  return { ...r, props };
}

const last = (fn: ReturnType<typeof vi.fn>) => fn.mock.calls[fn.mock.calls.length - 1][0] as Selection;

describe('FileList', () => {
  it('renders an accessible grid and virtualizes large lists', () => {
    setup(10_000);
    const grid = screen.getByRole('grid', { name: 'test list' });
    expect(grid).toHaveAttribute('aria-rowcount', '10000');
    const rows = screen.getAllByRole('row').filter((r) => r.hasAttribute('aria-rowindex'));
    expect(rows.length).toBeGreaterThan(10);
    expect(rows.length).toBeLessThan(100);
  });

  it('click, ctrl-click and shift-click update the selection', async () => {
    const { props, rerender } = setup();
    await fireEvent.click(screen.getByText('file1.txt'));
    let sel = last(props.onselectionchange);
    expect([...sel.selected]).toEqual(['/d/file1.txt']);

    await rerender({ selection: sel });
    await fireEvent.click(screen.getByText('file3.txt'), { shiftKey: true });
    sel = last(props.onselectionchange);
    expect([...sel.selected].sort()).toEqual(['/d/file1.txt', '/d/file2.txt', '/d/file3.txt']);

    await rerender({ selection: sel });
    await fireEvent.click(screen.getByText('file2.txt'), { ctrlKey: true });
    sel = last(props.onselectionchange);
    expect([...sel.selected].sort()).toEqual(['/d/file1.txt', '/d/file3.txt']);

    await rerender({ selection: sel });
    const selectedRows = screen.getAllByRole('row').filter((r) => r.getAttribute('aria-selected') === 'true');
    expect(selectedRows).toHaveLength(2);
  });

  it('keyboard navigation, open, select all and actions', async () => {
    const { props, rerender } = setup();
    const grid = screen.getByRole('grid');
    await fireEvent.keyDown(grid, { key: 'ArrowDown' });
    let sel = last(props.onselectionchange);
    expect(sel.focus).toBe('/d/file0.txt');
    await rerender({ selection: sel });
    expect(grid.getAttribute('aria-activedescendant')).toMatch(/-r0$/);

    await fireEvent.keyDown(grid, { key: 'ArrowDown', shiftKey: true });
    sel = last(props.onselectionchange);
    expect(sel.selected.size).toBe(2);
    await rerender({ selection: sel });

    await fireEvent.keyDown(grid, { key: 'Enter' });
    expect(props.onopen).toHaveBeenCalledWith(expect.objectContaining({ name: 'file1.txt' }));

    await fireEvent.keyDown(grid, { key: 'a', ctrlKey: true });
    expect(last(props.onselectionchange).selected.size).toBe(5);

    await fireEvent.keyDown(grid, { key: 'Delete' });
    await fireEvent.keyDown(grid, { key: 'F2' });
    await fireEvent.keyDown(grid, { key: 'Backspace' });
    expect(props.onkeyaction.mock.calls.map((c) => c[0])).toEqual(['delete', 'rename']);
    expect(props.onup).toHaveBeenCalled();
  });

  it('sortable column headers and empty state', async () => {
    const { props } = setup(0);
    expect(screen.getByText('empty')).toBeInTheDocument();
    const name = screen.getByRole('columnheader', { name: /name/i });
    expect(name).toHaveAttribute('aria-sort', 'ascending');
    await fireEvent.click(screen.getByRole('button', { name: /size/i }));
    expect(props.onsort).toHaveBeenCalledWith('size');
  });

  it('double click opens', async () => {
    const { props } = setup();
    await fireEvent.dblClick(screen.getByText('file0.txt'));
    expect(props.onopen).toHaveBeenCalledWith(expect.objectContaining({ kind: 'dir' }));
  });
});
