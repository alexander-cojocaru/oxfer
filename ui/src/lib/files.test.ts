import { describe, expect, it } from 'vitest';
import { baseName, breadcrumbs, filterEntries, joinPath, nextSort, parentPath, sortEntries, validateName } from './files';
import { formatBytes, formatDuration, formatPermissions } from './format';
import type { Entry } from './types';

const e = (name: string, kind: Entry['kind'] = 'file', size = 0, modified = 0): Entry => ({
  name,
  path: `/d/${name}`,
  kind,
  size,
  modified,
  permissions: 0o644,
});

describe('sorting and filtering', () => {
  const list = [e('b.txt', 'file', 5, 3), e('A', 'dir'), e('file10', 'file', 1, 1), e('file2', 'file', 9, 2), e('.hidden')];

  it('sorts dirs first, natural name order', () => {
    expect(sortEntries(list, { key: 'name', dir: 'asc' }).map((x) => x.name)).toEqual([
      'A',
      '.hidden',
      'b.txt',
      'file2',
      'file10',
    ]);
    expect(sortEntries(list, { key: 'size', dir: 'desc' }).map((x) => x.name)).toEqual([
      'A',
      'file2',
      'b.txt',
      'file10',
      '.hidden',
    ]);
  });

  it('filters hidden files and wildcards', () => {
    expect(filterEntries(list, '', false)).toHaveLength(4);
    expect(filterEntries(list, '', true)).toHaveLength(5);
    expect(filterEntries(list, 'FILE', false).map((x) => x.name)).toEqual(['file10', 'file2']);
    expect(filterEntries(list, 'file?', false).map((x) => x.name)).toEqual(['file2']);
    expect(filterEntries(list, '*.txt', false).map((x) => x.name)).toEqual(['b.txt']);
  });

  it('toggles sort direction', () => {
    expect(nextSort({ key: 'name', dir: 'asc' }, 'name')).toEqual({ key: 'name', dir: 'desc' });
    expect(nextSort({ key: 'name', dir: 'asc' }, 'size')).toEqual({ key: 'size', dir: 'desc' });
  });
});

describe('paths', () => {
  it('joins, splits and builds breadcrumbs', () => {
    expect(joinPath('/', 'a')).toBe('/a');
    expect(joinPath('/a/', 'b')).toBe('/a/b');
    expect(parentPath('/a/b')).toBe('/a');
    expect(parentPath('/a')).toBe('/');
    expect(parentPath('/')).toBeNull();
    expect(parentPath('C:/Users')).toBe('C:/');
    expect(baseName('/a/b.txt')).toBe('b.txt');
    expect(breadcrumbs('/home/u')).toEqual([
      { label: '/', path: '/' },
      { label: 'home', path: '/home' },
      { label: 'u', path: '/home/u' },
    ]);
    expect(breadcrumbs('C:/x').map((c) => c.path)).toEqual(['C:', 'C:/x']);
  });

  it('validates names', () => {
    expect(validateName('ok.txt')).toBeNull();
    expect(validateName(' ')).toBe('nameEmpty');
    expect(validateName('..')).toBe('nameReserved');
    expect(validateName('a/b')).toBe('nameSlash');
  });
});

describe('format', () => {
  it('formats values', () => {
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(1536)).toBe('1.5 KB');
    expect(formatDuration(75)).toBe('1m 15s');
    expect(formatDuration(null)).toBe('—');
    expect(formatPermissions(0o755, 'dir')).toBe('drwxr-xr-x');
  });
});
