import type { Profile } from './types';

export interface ProfileGroup {
  /** `null` for profiles without a group (listed first, without a heading). */
  name: string | null;
  profiles: Profile[];
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

/** Profiles matching `query`, grouped: ungrouped first, then groups by name. Empty groups are left out. */
export function groupProfiles(profiles: readonly Profile[], query = ''): ProfileGroup[] {
  const groups = new Map<string | null, Profile[]>();
  for (const p of profiles) {
    if (!matchesProfile(p, query)) continue;
    const key = p.group || null;
    groups.set(key, [...(groups.get(key) ?? []), p]);
  }
  return [...groups.entries()]
    .sort(([a], [b]) => (a === null ? -1 : b === null ? 1 : byName(a, b)))
    .map(([name, list]) => ({ name, profiles: list.sort((a, b) => byName(a.name, b.name)) }));
}

/** Distinct group names, sorted. */
export function groupNames(profiles: readonly Profile[]): string[] {
  return [...new Set(profiles.map((p) => p.group).filter((g): g is string => !!g))].sort(byName);
}
