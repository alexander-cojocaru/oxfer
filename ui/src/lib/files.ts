import type { Entry } from './types';

export type SortKey = 'name' | 'size' | 'modified';
export type SortDir = 'asc' | 'desc';

export interface SortSpec {
  key: SortKey;
  dir: SortDir;
}

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

const isDirLike = (e: Entry) => e.kind === 'dir';

/** Directories always first; then by the chosen column, ties broken by name. */
export function sortEntries(entries: readonly Entry[], spec: SortSpec): Entry[] {
  const sign = spec.dir === 'asc' ? 1 : -1;
  return [...entries].sort((a, b) => {
    const da = isDirLike(a);
    const db = isDirLike(b);
    if (da !== db) return da ? -1 : 1;
    let c = 0;
    if (spec.key === 'size') c = a.size - b.size;
    else if (spec.key === 'modified') c = (a.modified ?? 0) - (b.modified ?? 0);
    if (c === 0) c = collator.compare(a.name, b.name);
    return c * sign;
  });
}

export function isHidden(e: Entry): boolean {
  return e.name.startsWith('.');
}

/** Case-insensitive substring filter; `*` and `?` work as wildcards. */
export function filterEntries(entries: readonly Entry[], query: string, showHidden: boolean): Entry[] {
  const q = query.trim().toLowerCase();
  let match: (name: string) => boolean = () => true;
  if (q.includes('*') || q.includes('?')) {
    const re = new RegExp(
      '^' +
        q
          .split('')
          .map((ch) => (ch === '*' ? '.*' : ch === '?' ? '.' : ch.replace(/[.+^${}()|[\]\\]/g, '\\$&')))
          .join('') +
        '$',
    );
    match = (name) => re.test(name.toLowerCase());
  } else if (q) {
    match = (name) => name.toLowerCase().includes(q);
  }
  return entries.filter((e) => (showHidden || !isHidden(e)) && match(e.name));
}

export function nextSort(current: SortSpec, key: SortKey): SortSpec {
  if (current.key === key) return { key, dir: current.dir === 'asc' ? 'desc' : 'asc' };
  return { key, dir: key === 'name' ? 'asc' : 'desc' };
}

// ---- POSIX-style path helpers (the backend uses `/` everywhere) ----

export function joinPath(dir: string, name: string): string {
  if (name.startsWith('/')) return name;
  if (dir === '' || dir === '/') return `/${name}`;
  return `${dir.replace(/\/+$/, '')}/${name}`;
}

export function parentPath(path: string): string | null {
  const trimmed = path.replace(/\/+$/, '');
  if (trimmed === '' || trimmed === '/') return null;
  const idx = trimmed.lastIndexOf('/');
  if (idx < 0) return null;
  if (idx === 0) return '/';
  const parent = trimmed.slice(0, idx);
  // Windows drive root like "C:"
  return /^[A-Za-z]:$/.test(parent) ? `${parent}/` : parent;
}

export function baseName(path: string): string {
  const trimmed = path.replace(/\/+$/, '');
  const idx = trimmed.lastIndexOf('/');
  return idx < 0 ? trimmed : trimmed.slice(idx + 1);
}

export interface Crumb {
  label: string;
  path: string;
}

export function breadcrumbs(path: string): Crumb[] {
  const parts = path.split('/').filter(Boolean);
  const absolute = path.startsWith('/');
  const crumbs: Crumb[] = absolute ? [{ label: '/', path: '/' }] : [];
  let acc = absolute ? '' : '';
  parts.forEach((p, i) => {
    acc = i === 0 && !absolute ? p : `${acc}/${p}`;
    crumbs.push({ label: p, path: acc });
  });
  return crumbs;
}

/** Validate a single file name typed by the user; returns an error key or null. */
export function validateName(name: string): string | null {
  const n = name.trim();
  if (!n) return 'nameEmpty';
  if (n === '.' || n === '..') return 'nameReserved';
  if (n.includes('/') || n.includes('\0')) return 'nameSlash';
  return null;
}
