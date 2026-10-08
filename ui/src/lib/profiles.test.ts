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
    expect(g).toEqual([{ name: 'ACME', profiles: [all[3]] }]);
    expect(groupProfiles(all, 'nothing')).toEqual([]);
  });

  it('lists distinct group names', () => {
    expect(groupNames(all)).toEqual(['ACME', 'Work', 'work']);
  });
});
