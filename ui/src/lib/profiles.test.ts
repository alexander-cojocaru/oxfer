import { describe, expect, it } from 'vitest';
import { groupNames, groupProfiles, matchesProfile } from './profiles';
import type { Profile } from './types';

function prof(name: string, group: string | null, extra: Partial<Profile> = {}): Profile {
  return {
    name,
    protocol: 'sftp',
    host: `${name}.example.com`,
    port: null,
    user: null,
    auth: { method: 'agent' },
    remoteDir: null,
    group,
    credential: null,
    ...extra,
  };
}

const all = [
  prof('web', 'Work'),
  prof('db', 'work', { user: 'postgres' }),
  prof('nas', null, { remoteDir: '/volume1/photos' }),
  prof('shop', 'ACME'),
  prof('Alpha', null),
];

describe('profiles', () => {
  it('matches every term in any field, ignoring case', () => {
    const db = all[1];
    expect(matchesProfile(db, '')).toBe(true);
    expect(matchesProfile(db, 'DB')).toBe(true);
    expect(matchesProfile(db, 'postgres  work')).toBe(true);
    expect(matchesProfile(db, 'postgres acme')).toBe(false);
    expect(matchesProfile(all[2], 'photos')).toBe(true);
  });

  it('groups with ungrouped first, then groups and names alphabetically', () => {
    const g = groupProfiles(all);
    expect(g.map((x) => x.name)).toEqual([null, 'ACME', 'Work', 'work']);
    expect(g[0].profiles.map((p) => p.name)).toEqual(['Alpha', 'nas']);
  });

  it('leaves out groups without matches', () => {
    const g = groupProfiles(all, 'shop');
    expect(g).toEqual([{ name: 'ACME', label: 'ACME', depth: 0, profiles: [all[3]], total: 1 }]);
    expect(groupProfiles(all, 'nothing')).toEqual([]);
  });

  it('nests sub groups under their parent, parents without profiles included', () => {
    const list = [
      prof('a', 'Work/Web'),
      prof('b', 'Work'),
      prof('c', 'Work/Web/Prod'),
      prof('d', 'Customers/ACME'),
      prof('e', 'Work b'),
    ];
    const g = groupProfiles(list);
    expect(g.map((x) => [x.name, x.depth, x.profiles.length, x.total])).toEqual([
      ['Customers', 0, 0, 1],
      ['Customers/ACME', 1, 1, 1],
      ['Work', 0, 1, 3],
      ['Work/Web', 1, 1, 2],
      ['Work/Web/Prod', 2, 1, 1],
      ['Work b', 0, 1, 1],
    ]);
    expect(g[1].label).toBe('ACME');
    expect(groupProfiles(list, 'prod').map((x) => x.name)).toEqual(['Work', 'Work/Web', 'Work/Web/Prod']);
    expect(groupNames(list)).toEqual(['Customers', 'Customers/ACME', 'Work', 'Work/Web', 'Work/Web/Prod', 'Work b']);
  });

  it('has no depth limit', () => {
    const path = Array.from({ length: 50 }, (_, i) => `g${i}`).join('/');
    const g = groupProfiles([prof('deep', path)]);
    expect(g).toHaveLength(50);
    expect(g[49]).toMatchObject({ name: path, label: 'g49', depth: 49, total: 1 });
    expect(g[0]).toMatchObject({ name: 'g0', depth: 0, total: 1 });
  });

  it('lists distinct group names', () => {
    expect(groupNames(all)).toEqual(['ACME', 'Work', 'work']);
  });
});
