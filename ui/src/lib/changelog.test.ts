import { describe, expect, it } from 'vitest';
import { parseChangelog } from './changelog';

describe('parseChangelog', () => {
  it('parses releases, dates and bullets', () => {
    const r = parseChangelog('# Changelog\n\nintro\n\n## 1.1.0 - 2026-01-02\n\n- a\n- b wraps\n  here\n\n## [1.0.0]\n- c\n');
    expect(r).toEqual([
      { version: '1.1.0', date: '2026-01-02', items: ['a', 'b wraps here'] },
      { version: '1.0.0', date: '', items: ['c'] },
    ]);
  });

  it('ignores text before the first release', () => {
    expect(parseChangelog('# Changelog\n- stray\n')).toEqual([]);
  });
});
