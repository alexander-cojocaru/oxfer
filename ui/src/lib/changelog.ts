// Parses CHANGELOG.md (Keep-a-Changelog-ish: `## <version> - <date>` headings
// followed by `- item` bullets) into data the UI can render.

export interface ChangelogRelease {
  version: string;
  date: string;
  items: string[];
}

export function parseChangelog(src: string): ChangelogRelease[] {
  const releases: ChangelogRelease[] = [];
  let current: ChangelogRelease | null = null;
  for (const raw of src.split(/\r?\n/)) {
    const line = raw.trim();
    const head = /^##\s+\[?([^\]\s]+)\]?(?:\s+[-–]\s+(.+))?$/.exec(line);
    if (head) {
      current = { version: head[1], date: head[2] ?? '', items: [] };
      releases.push(current);
    } else if (current && /^[-*]\s+/.test(line)) {
      current.items.push(line.replace(/^[-*]\s+/, ''));
    } else if (current && line && current.items.length > 0 && !line.startsWith('#')) {
      // continuation of a wrapped bullet
      current.items[current.items.length - 1] += ` ${line}`;
    }
  }
  return releases;
}
