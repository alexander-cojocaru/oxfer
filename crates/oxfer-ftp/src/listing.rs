//! Parsers for FTP directory listings: MLSD/MLST (RFC 3659) and the de-facto
//! `LIST` formats (unix `ls -l` and DOS/IIS).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use oxfer_core::{Entry, EntryKind, RemotePath};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEntry {
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub permissions: Option<u32>,
    pub link_target: Option<String>,
}

impl ParsedEntry {
    pub fn into_entry(self, path: RemotePath) -> Entry {
        Entry {
            path,
            kind: self.kind,
            size: self.size,
            modified: self.modified,
            permissions: self.permissions,
        }
    }
}

fn is_dot(name: &str) -> bool {
    name == "." || name == ".."
}

// ---------------------------------------------------------------- dates

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn year_of(t: SystemTime) -> i64 {
    let days = match t.duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs() / 86_400) as i64,
        Err(_) => 0,
    };
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    yoe + era * 400 + i64::from(m <= 2)
}

fn to_time(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> Option<SystemTime> {
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let secs = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s;
    u64::try_from(secs)
        .ok()
        .map(|s| UNIX_EPOCH + Duration::from_secs(s))
}

fn month_number(s: &str) -> Option<i64> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let lower = s.to_ascii_lowercase();
    MONTHS
        .iter()
        .position(|m| *m == lower)
        .map(|i| i as i64 + 1)
}

fn parse_hm(s: &str) -> Option<(i64, i64)> {
    let (h, m) = s.split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    Some((h.parse().ok()?, m.parse().ok()?))
}

// ---------------------------------------------------------------- MLSx

/// Parse one MLSD/MLST fact line (`type=file;size=1;modify=...; name`).
/// The `cdir`/`pdir` flags are returned as `Dir` with `is_dot_entry` set.
fn parse_facts(line: &str) -> Option<(ParsedEntry, bool)> {
    let line = line.trim_end_matches(['\r', '\n']);
    let (facts, name) = line.split_once(' ')?;
    if name.is_empty() {
        return None;
    }
    let mut kind = EntryKind::Other;
    let mut dot = false;
    let (mut size, mut modified, mut permissions, mut link_target) = (0, None, None, None);
    for fact in facts.split(';').filter(|f| !f.is_empty()) {
        let (k, v) = fact.split_once('=')?;
        match k.to_ascii_lowercase().as_str() {
            "type" => {
                let lower = v.to_ascii_lowercase();
                kind = match lower.as_str() {
                    "file" => EntryKind::File,
                    "dir" => EntryKind::Dir,
                    "cdir" | "pdir" => {
                        dot = true;
                        EntryKind::Dir
                    }
                    t if t.starts_with("os.unix=slink") || t.starts_with("os.unix=symlink") => {
                        // Keep the original case for the target.
                        link_target = v.split_once(':').map(|(_, t)| t.to_string());
                        EntryKind::Symlink
                    }
                    _ => EntryKind::Other,
                };
            }
            "size" | "sizd" => size = v.parse().unwrap_or(0),
            "modify" => modified = parse_mlsx_time(v),
            "unix.mode" => {
                permissions = u32::from_str_radix(v, 8).ok().map(|m| m & 0o7777);
            }
            _ => {}
        }
    }
    Some((
        ParsedEntry {
            name: name.to_string(),
            kind,
            size,
            modified,
            permissions,
            link_target,
        },
        dot,
    ))
}

/// `YYYYMMDDHHMMSS[.sss]`, always UTC.
fn parse_mlsx_time(v: &str) -> Option<SystemTime> {
    let v = v.split('.').next()?;
    if v.len() != 14 || !v.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n = |r: std::ops::Range<usize>| v[r].parse::<i64>().ok();
    to_time(
        n(0..4)?,
        n(4..6)?,
        n(6..8)?,
        n(8..10)?,
        n(10..12)?,
        n(12..14)?,
    )
}

/// Parse the lines of an MLSD response, dropping `.`/`..`/`cdir`/`pdir`.
pub fn parse_mlsd(lines: &[String]) -> Vec<ParsedEntry> {
    lines
        .iter()
        .filter_map(|l| parse_facts(l))
        .filter(|(e, dot)| !dot && !is_dot(&e.name))
        .map(|(e, _)| e)
        .collect()
}

/// Parse the single fact line of an MLST response.
pub fn parse_mlst(line: &str) -> Option<ParsedEntry> {
    parse_facts(line).map(|(e, _)| e)
}

// ---------------------------------------------------------------- LIST

/// Whitespace-separated tokens with their byte offsets.
fn tokens(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in s.char_indices() {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(st)) => {
                out.push((st, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(st) = start {
        out.push((st, s.len()));
    }
    out
}

/// Text after the single separator character following byte offset `end`.
fn rest_after(s: &str, end: usize) -> Option<&str> {
    let mut chars = s[end..].chars();
    let sep = chars.next()?;
    sep.is_whitespace().then_some(chars.as_str())
}

fn parse_mode(perms: &str) -> Option<u32> {
    let b = perms.as_bytes();
    if b.len() < 10 {
        return None;
    }
    let mut mode = 0;
    for (i, shift) in [(1, 6), (4, 3), (7, 0)] {
        if b[i] == b'r' {
            mode |= 0o4 << shift;
        }
        if b[i + 1] == b'w' {
            mode |= 0o2 << shift;
        }
        if matches!(b[i + 2], b'x' | b's' | b't') {
            mode |= 0o1 << shift;
        }
    }
    if matches!(b[3], b's' | b'S') {
        mode |= 0o4000;
    }
    if matches!(b[6], b's' | b'S') {
        mode |= 0o2000;
    }
    if matches!(b[9], b't' | b'T') {
        mode |= 0o1000;
    }
    Some(mode)
}

/// Resolve a year-less `Mon DD HH:MM` date: `ls` prints those for files
/// modified within roughly the last six months, so a date in the future
/// belongs to the previous year.
fn resolve_recent(mo: i64, d: i64, h: i64, mi: i64, now: SystemTime) -> Option<SystemTime> {
    let year = year_of(now);
    let t = to_time(year, mo, d, h, mi, 0)?;
    if t > now + Duration::from_secs(2 * 86_400) {
        to_time(year - 1, mo, d, h, mi, 0)
    } else {
        Some(t)
    }
}

fn parse_unix_line(line: &str, now: SystemTime) -> Option<ParsedEntry> {
    let toks = tokens(line);
    let text = |i: usize| &line[toks[i].0..toks[i].1];
    if toks.len() < 7 {
        return None;
    }
    let perms = text(0);
    let kind = match perms.as_bytes()[0] {
        b'-' => EntryKind::File,
        b'd' => EntryKind::Dir,
        b'l' => EntryKind::Symlink,
        b'b' | b'c' | b'p' | b's' => EntryKind::Other,
        _ => return None,
    };
    if perms.len() < 10 || !perms.bytes().skip(1).all(|b| b"rwxsStT-+@.".contains(&b)) {
        return None;
    }
    // Locate the date: "Mon DD HH:MM|YYYY" or "YYYY-MM-DD HH:MM". The size is
    // the numeric token right before it; owner/group columns may vary.
    for i in 4..toks.len().saturating_sub(1) {
        let size_tok = text(i - 1).trim_end_matches(',');
        if size_tok.is_empty() || !size_tok.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let (modified, name_from) = if let Some(mo) = month_number(text(i)) {
            if i + 2 >= toks.len() {
                continue;
            }
            let Ok(day) = text(i + 1).parse::<i64>() else {
                continue;
            };
            let last = text(i + 2);
            let t = if let Some((h, mi)) = parse_hm(last) {
                resolve_recent(mo, day, h, mi, now)
            } else if last.len() == 4 && last.bytes().all(|b| b.is_ascii_digit()) {
                to_time(last.parse().ok()?, mo, day, 0, 0, 0)
            } else {
                continue;
            };
            (t, toks[i + 2].1)
        } else if let (Some((ys, rest)), Some((h, mi))) =
            (text(i).split_once('-'), parse_hm(text(i + 1)))
        {
            let Some((ms, ds)) = rest.split_once('-') else {
                continue;
            };
            let (Ok(y), Ok(m), Ok(d)) = (ys.parse(), ms.parse(), ds.parse()) else {
                continue;
            };
            (to_time(y, m, d, h, mi, 0), toks[i + 1].1)
        } else {
            continue;
        };
        let rest = rest_after(line, name_from)?;
        if rest.is_empty() {
            return None;
        }
        let size = size_tok.parse().unwrap_or(0);
        let (name, link_target) = match (kind, rest.split_once(" -> ")) {
            (EntryKind::Symlink, Some((n, t))) => (n.to_string(), Some(t.to_string())),
            _ => (rest.to_string(), None),
        };
        return Some(ParsedEntry {
            name,
            kind,
            size: if kind == EntryKind::Dir { 0 } else { size },
            modified,
            permissions: parse_mode(perms),
            link_target,
        });
    }
    None
}

fn parse_dos_line(line: &str) -> Option<ParsedEntry> {
    let toks = tokens(line);
    if toks.len() < 4 {
        return None;
    }
    let text = |i: usize| &line[toks[i].0..toks[i].1];
    let mut date = text(0).split('-');
    let (m, d, y) = (date.next()?, date.next()?, date.next()?);
    if date.next().is_some() || !(y.len() == 2 || y.len() == 4) {
        return None;
    }
    let (m, d, mut y): (i64, i64, i64) = (m.parse().ok()?, d.parse().ok()?, y.parse().ok()?);
    if text(0).split('-').nth(2)?.len() == 2 {
        y += if y < 70 { 2000 } else { 1900 };
    }
    let time = text(1);
    let (clock, pm) = match time.len().checked_sub(2).map(|i| time.split_at(i)) {
        Some((c, s)) if s.eq_ignore_ascii_case("am") => (c, Some(false)),
        Some((c, s)) if s.eq_ignore_ascii_case("pm") => (c, Some(true)),
        _ => (time, None),
    };
    let (mut h, mi) = parse_hm(clock)?;
    match pm {
        Some(true) if h < 12 => h += 12,
        Some(false) if h == 12 => h = 0,
        _ => {}
    }
    let (kind, size) = if text(2).eq_ignore_ascii_case("<dir>") {
        (EntryKind::Dir, 0)
    } else {
        let digits: String = text(2).chars().filter(|c| *c != ',').collect();
        (EntryKind::File, digits.parse().ok()?)
    };
    let name = line[toks[2].1..].trim_start();
    if name.is_empty() {
        return None;
    }
    Some(ParsedEntry {
        name: name.to_string(),
        kind,
        size,
        modified: to_time(y, m, d, h, mi, 0),
        permissions: None,
        link_target: None,
    })
}

/// Parse one `LIST` line (unix or DOS style). `now` resolves year-less dates.
pub fn parse_list_line(line: &str, now: SystemTime) -> Option<ParsedEntry> {
    let line = line.trim_end_matches(['\r', '\n']);
    parse_unix_line(line, now).or_else(|| parse_dos_line(line))
}

/// Parse a whole `LIST` response, skipping `total` lines, `.`/`..` and junk.
pub fn parse_list(lines: &[String], now: SystemTime) -> Vec<ParsedEntry> {
    lines
        .iter()
        .filter_map(|l| parse_list_line(l, now))
        .filter(|e| !is_dot(&e.name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> SystemTime {
        to_time(y, mo, d, h, mi, 0).unwrap()
    }

    /// 2024-06-15 12:00 UTC
    fn now() -> SystemTime {
        at(2024, 6, 15, 12, 0)
    }

    #[test]
    fn civil_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(year_of(now()), 2024);
        assert_eq!(year_of(UNIX_EPOCH), 1970);
        assert_eq!(year_of(at(2023, 12, 31, 23, 59)), 2023);
        assert_eq!(year_of(at(2024, 1, 1, 0, 0)), 2024);
        assert_eq!(year_of(at(2024, 2, 29, 0, 0)), 2024);
    }

    #[test]
    fn unix_file_with_time() {
        let e = parse_list_line(
            "-rw-r--r--    1 ftp      ftp          1234 Jan 05 12:34 readme.txt",
            now(),
        )
        .unwrap();
        assert_eq!(e.name, "readme.txt");
        assert_eq!(e.kind, EntryKind::File);
        assert_eq!(e.size, 1234);
        assert_eq!(e.permissions, Some(0o644));
        assert_eq!(e.modified, Some(at(2024, 1, 5, 12, 34)));
    }

    #[test]
    fn unix_file_with_year() {
        let e = parse_list_line("-rwxr-xr-x 1 root root 99 Dec  1  2019 old.sh", now()).unwrap();
        assert_eq!(e.name, "old.sh");
        assert_eq!(e.permissions, Some(0o755));
        assert_eq!(e.modified, Some(at(2019, 12, 1, 0, 0)));
    }

    #[test]
    fn unix_future_time_means_previous_year() {
        let e = parse_list_line("-rw-r--r-- 1 u g 5 Nov 20 08:15 a", now()).unwrap();
        assert_eq!(e.modified, Some(at(2023, 11, 20, 8, 15)));
        // Just ahead of "now" (clock skew) stays in the current year.
        let e = parse_list_line("-rw-r--r-- 1 u g 5 Jun 16 08:15 a", now()).unwrap();
        assert_eq!(e.modified, Some(at(2024, 6, 16, 8, 15)));
    }

    #[test]
    fn unix_dir() {
        let e = parse_list_line(
            "drwxr-xr-x   2 user  group      4096 Mar  3 09:00 My Documents",
            now(),
        )
        .unwrap();
        assert_eq!(e.name, "My Documents");
        assert_eq!(e.kind, EntryKind::Dir);
        assert_eq!(e.size, 0);
        assert!(e.link_target.is_none());
    }

    #[test]
    fn unix_names_with_spaces() {
        let e = parse_list_line(
            "-rw-r--r-- 1 u g 10 Jan  5 12:34  leading and  double  spaces ",
            now(),
        )
        .unwrap();
        assert_eq!(e.name, " leading and  double  spaces ");
        let e = parse_list_line(
            "-rw-r--r-- 1 u g 10 Jan  5 12:34 Jan 5 12:34 fake.txt",
            now(),
        )
        .unwrap();
        assert_eq!(e.name, "Jan 5 12:34 fake.txt");
    }

    #[test]
    fn unix_symlink() {
        let e = parse_list_line("lrwxrwxrwx 1 u g 7 Feb 14 2021 a -> b", now()).unwrap();
        assert_eq!(e.name, "a");
        assert_eq!(e.kind, EntryKind::Symlink);
        assert_eq!(e.link_target.as_deref(), Some("b"));
        assert_eq!(e.permissions, Some(0o777));

        let e = parse_list_line(
            "lrwxrwxrwx 1 u g 7 Feb 14 2021 my link -> ../other dir/x",
            now(),
        )
        .unwrap();
        assert_eq!(e.name, "my link");
        assert_eq!(e.link_target.as_deref(), Some("../other dir/x"));
    }

    #[test]
    fn unix_arrow_in_regular_file_name_is_kept() {
        let e = parse_list_line("-rw-r--r-- 1 u g 7 Feb 14 2021 a -> b", now()).unwrap();
        assert_eq!(e.name, "a -> b");
        assert_eq!(e.kind, EntryKind::File);
    }

    #[test]
    fn unix_special_bits_and_acl_marker() {
        let e = parse_list_line("-rwsr-sr-t+ 1 u g 7 Feb 14 2021 x", now()).unwrap();
        assert_eq!(e.permissions, Some(0o7755));
        assert_eq!(e.name, "x");
    }

    #[test]
    fn unix_without_group_column() {
        let e = parse_list_line("-rw-r--r-- 1 owner 42 Jan  5 12:34 nogroup.txt", now()).unwrap();
        assert_eq!(e.name, "nogroup.txt");
        assert_eq!(e.size, 42);
    }

    #[test]
    fn unix_iso_dates() {
        let e = parse_list_line("-rw-r--r-- 1 u g 42 2023-07-09 18:30 iso.txt", now()).unwrap();
        assert_eq!(e.name, "iso.txt");
        assert_eq!(e.modified, Some(at(2023, 7, 9, 18, 30)));
    }

    #[test]
    fn unix_device_file() {
        let e = parse_list_line("crw-rw-rw- 1 root root 1, 3 Jan  1 00:00 null", now()).unwrap();
        assert_eq!(e.kind, EntryKind::Other);
        assert_eq!(e.name, "null");
    }

    #[test]
    fn unix_unicode_name() {
        let e = parse_list_line("-rw-r--r-- 1 u g 1 Jan  5 12:34 grüße 日本.txt", now()).unwrap();
        assert_eq!(e.name, "grüße 日本.txt");
    }

    #[test]
    fn dos_style() {
        let e = parse_list_line(
            "01-05-24  03:45PM       <DIR>          Program Files",
            now(),
        )
        .unwrap();
        assert_eq!(e.name, "Program Files");
        assert_eq!(e.kind, EntryKind::Dir);
        assert_eq!(e.modified, Some(at(2024, 1, 5, 15, 45)));

        let e =
            parse_list_line("12-31-1999  12:00AM              1,234 my file.txt", now()).unwrap();
        assert_eq!(e.name, "my file.txt");
        assert_eq!(e.kind, EntryKind::File);
        assert_eq!(e.size, 1234);
        assert_eq!(e.modified, Some(at(1999, 12, 31, 0, 0)));

        let e = parse_list_line("06-01-98  11:59AM 5 old", now()).unwrap();
        assert_eq!(e.modified, Some(at(1998, 6, 1, 11, 59)));
    }

    #[test]
    fn list_skips_junk_and_dots() {
        let lines: Vec<String> = [
            "total 12",
            "drwxr-xr-x 2 u g 4096 Jan  5 12:34 .",
            "drwxr-xr-x 2 u g 4096 Jan  5 12:34 ..",
            "-rw-r--r-- 1 u g 1 Jan  5 12:34 a",
            "",
            "garbage line",
        ]
        .map(String::from)
        .to_vec();
        let out = parse_list(&lines, now());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "a");
    }

    #[test]
    fn rejects_non_listing_lines() {
        assert!(parse_list_line("total 8", now()).is_none());
        assert!(parse_list_line("", now()).is_none());
        assert!(parse_list_line("-rw-r--r-- 1 u g notasize Jan 5 12:34 a", now()).is_none());
        assert!(parse_list_line("-rw-r--r-- 1 u g 1 Foo 5 12:34 a", now()).is_none());
    }

    #[test]
    fn mlsd_basic() {
        let e = parse_facts("type=file;size=1234;modify=20240105123456;perm=adfr; a file.txt")
            .unwrap()
            .0;
        assert_eq!(e.name, "a file.txt");
        assert_eq!(e.kind, EntryKind::File);
        assert_eq!(e.size, 1234);
        assert_eq!(e.modified, Some(to_time(2024, 1, 5, 12, 34, 56).unwrap()));
        assert_eq!(e.permissions, None);
    }

    #[test]
    fn mlsd_case_fraction_and_unix_mode() {
        let e = parse_mlst("Type=DIR;Modify=20240105123456.789;UNIX.mode=0755; sub").unwrap();
        assert_eq!(e.kind, EntryKind::Dir);
        assert_eq!(e.permissions, Some(0o755));
        assert_eq!(e.modified, Some(to_time(2024, 1, 5, 12, 34, 56).unwrap()));
    }

    #[test]
    fn mlsd_symlink_and_other() {
        let e = parse_mlst("type=OS.unix=slink:/etc/Target;size=3; lnk").unwrap();
        assert_eq!(e.kind, EntryKind::Symlink);
        assert_eq!(e.link_target.as_deref(), Some("/etc/Target"));
        let e = parse_mlst("type=OS.unix=symlink;size=3; lnk2").unwrap();
        assert_eq!(e.kind, EntryKind::Symlink);
        let e = parse_mlst("type=OS.unix=socket; s").unwrap();
        assert_eq!(e.kind, EntryKind::Other);
    }

    #[test]
    fn mlsd_filters_dot_entries() {
        let lines: Vec<String> = [
            "type=cdir;modify=20240101000000; .",
            "type=pdir;modify=20240101000000; ..",
            "type=cdir;modify=20240101000000; /full/path",
            "type=dir;modify=20240101000000; ..",
            "type=file;size=1; keep",
            "bogus",
        ]
        .map(String::from)
        .to_vec();
        let out = parse_mlsd(&lines);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "keep");
    }

    #[test]
    fn mlst_accepts_cdir() {
        let e = parse_mlst("type=cdir;modify=20240101000000; /pub").unwrap();
        assert_eq!(e.kind, EntryKind::Dir);
        assert_eq!(e.name, "/pub");
    }

    #[test]
    fn into_entry() {
        let p = parse_list_line("-rw-r--r-- 1 u g 7 Feb 14 2021 f", now()).unwrap();
        let e = p.into_entry(RemotePath::new("/d/f"));
        assert_eq!(e.name(), "f");
        assert_eq!(e.size, 7);
    }
}
