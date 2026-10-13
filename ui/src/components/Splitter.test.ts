import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import Splitter from './Splitter.svelte';

function setup(orientation: 'vertical' | 'horizontal' = 'vertical') {
  const onmove = vi.fn();
  const onstep = vi.fn();
  const onreset = vi.fn();
  render(Splitter, { orientation, label: 'Resize', onmove, onstep, onreset });
  return { sep: screen.getByRole('separator', { name: 'Resize' }), onmove, onstep, onreset };
}

describe('Splitter', () => {
  it('reports pointer position only while dragging', async () => {
    const { sep, onmove } = setup();
    await fireEvent.pointerMove(sep, { clientX: 50 });
    expect(onmove).not.toHaveBeenCalled();
    await fireEvent.pointerDown(sep, { clientX: 10, button: 0 });
    await fireEvent.pointerMove(sep, { clientX: 120 });
    expect(onmove).toHaveBeenLastCalledWith(120);
    await fireEvent.pointerUp(sep);
    await fireEvent.pointerMove(sep, { clientX: 200 });
    expect(onmove).toHaveBeenCalledTimes(1);
  });

  it('uses clientY for horizontal splitters', async () => {
    const { sep, onmove } = setup('horizontal');
    await fireEvent.pointerDown(sep, { button: 0 });
    await fireEvent.pointerMove(sep, { clientX: 5, clientY: 77 });
    expect(onmove).toHaveBeenLastCalledWith(77);
  });

  it('supports keyboard steps and reset', async () => {
    const { sep, onstep, onreset } = setup();
    await fireEvent.keyDown(sep, { key: 'ArrowRight' });
    expect(onstep).toHaveBeenLastCalledWith(1, false);
    await fireEvent.keyDown(sep, { key: 'ArrowLeft', shiftKey: true });
    expect(onstep).toHaveBeenLastCalledWith(-1, true);
    await fireEvent.dblClick(sep);
    expect(onreset).toHaveBeenCalledTimes(1);
  });
});
