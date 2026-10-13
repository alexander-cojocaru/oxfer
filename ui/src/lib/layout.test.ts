import { beforeEach, describe, expect, it } from 'vitest';
import { clamp, loadSize, saveSize, SIDEBAR } from './layout';

beforeEach(() => localStorage.clear());

describe('layout sizes', () => {
  it('clamps', () => {
    expect(clamp(5, 10, 20)).toBe(10);
    expect(clamp(25, 10, 20)).toBe(20);
    expect(clamp(15, 10, 20)).toBe(15);
  });

  it('falls back to the default and clamps stored values', () => {
    expect(loadSize(SIDEBAR)).toBe(SIDEBAR.def);
    saveSize(SIDEBAR.key, 9999);
    expect(loadSize(SIDEBAR)).toBe(SIDEBAR.max);
    localStorage.setItem(SIDEBAR.key, 'junk');
    expect(loadSize(SIDEBAR)).toBe(SIDEBAR.def);
  });
});
