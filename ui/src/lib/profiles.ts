import type { Profile } from './types';

/** Separator between a group and its sub groups: `Customers/ACME`. */
export const GROUP_SEP = '/';

export interface ProfileGroup {
  /** Full path (`Customers/ACME`); `null` for profiles without a group (listed first, without a heading). */
  name: string | null;
  /** Last path segment, shown as the heading. */
  label: string;
  /** Nesting level: 0 for a top-level group. */
  depth: number;
  /** Profiles directly in this group (not in its sub groups). */
  profiles: Profile[];
  /** Profiles in this group and all its sub groups. */
  total: number;
}

const segments = (name: string) => name.split(GROUP_SEP).filter(Boolean);

/** Every ancestor path of `name`, outermost first: `a/b/c` gives `a`, `a/b`. */
export function parentGroups(name: string): string[] {
  const parts = segments(name);
  return parts.slice(0, -1).map((_, i) => parts.slice(0, i + 1).join(GROUP_SEP));
}

const byName = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: 'base' });

/** Every whitespace-separated term must occur in the name, host, user, group or start directory. */
export function matchesProfile(p: Profile, query: string): boolean {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return true;
  const haystack = [p.name, p.host, p.user, p.group, p.remoteDir]
    .filter((s): s is string => !!s)
    .join('\n')
    .toLowerCase();
  return terms.every((t) => haystack.includes(t));
}

/** Segment-wise, so a group is followed by its sub groups rather than by `Work b`. */
function byPath(a: string, b: string): number {
  const [x, y] = [segments(a), segments(b)];
  for (let i = 0; i < Math.min(x.length, y.length); i++) {
    const c = byName(x[i], y[i]);
    if (c !== 0) return c;
  }
  return x.length - y.length;
}

/**
 * Profiles matching `query`, grouped: ungrouped first, then groups as a depth-first tree.
 * A group appears if it or a sub group has matches; parents without profiles of their own
 * are included so the tree stays connected.
 */
export function groupProfiles(profiles: readonly Profile[], query = ''): ProfileGroup[] {
  const own = new Map<string | null, Profile[]>();
  for (const p of profiles) {
    if (!matchesProfile(p, query)) continue;
    const key = p.group ? segments(p.group).join(GROUP_SEP) || null : null;
    own.set(key, [...(own.get(key) ?? []), p]);
  }
  const names = new Set<string>();
  const total = new Map<string, number>();
  for (const [name, list] of own) {
    if (name === null) continue;
    for (const n of [...parentGroups(name), name]) {
      names.add(n);
      total.set(n, (total.get(n) ?? 0) + list.length);
    }
  }
  const sortProfiles = (list: Profile[]) => list.sort((a, b) => byName(a.name, b.name));
  const out: ProfileGroup[] = [];
  if (own.has(null)) out.push({ name: null, label: '', depth: 0, profiles: sortProfiles(own.get(null)!), total: own.get(null)!.length });
  for (const name of [...names].sort(byPath)) {
    const parts = segments(name);
    out.push({
      name,
      label: parts[parts.length - 1],
      depth: parts.length - 1,
      profiles: sortProfiles(own.get(name) ?? []),
      total: total.get(name)!,
    });
  }
  return out;
}

/** Distinct group paths, sub groups and their parents included, sorted. */
export function groupNames(profiles: readonly Profile[]): string[] {
  const names = new Set<string>();
  for (const g of profiles.map((p) => p.group)) {
    if (g) for (const n of [...parentGroups(g), g]) names.add(n);
  }
  return [...names].sort(byPath);
}
