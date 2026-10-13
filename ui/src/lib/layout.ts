// Pane sizes the user can drag; persisted per viewer in localStorage.

export const SIDEBAR = { key: 'oxfer.sidebarWidth', def: 240, min: 160, max: 480 } as const;
export const SPLIT = { key: 'oxfer.paneSplit', def: 0.5, min: 0.2, max: 0.8 } as const;
export const QUEUE = { key: 'oxfer.queueHeight', def: 200, min: 80, max: 800 } as const;

export function clamp(v: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, v));
}

export function loadSize(spec: { key: string; def: number; min: number; max: number }): number {
  try {
    const raw = localStorage.getItem(spec.key);
    const v = raw === null ? NaN : Number(raw);
    return Number.isFinite(v) ? clamp(v, spec.min, spec.max) : spec.def;
  } catch {
    return spec.def;
  }
}

export function saveSize(key: string, value: number): void {
  try {
    localStorage.setItem(key, String(value));
  } catch {
    // storage unavailable: the size just won't persist
  }
}
