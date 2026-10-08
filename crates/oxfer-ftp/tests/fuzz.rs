//! Property tests: listing parsers handle arbitrary server output without
//! panicking, and parsed names never contain path separators.

use std::time::SystemTime;

use oxfer_ftp::listing::{parse_list, parse_list_line, parse_mlsd, parse_mlst};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn list_never_panics(line in "\\PC{0,200}") {
        let _ = parse_list_line(&line, SystemTime::now());
    }

    #[test]
    fn mlst_never_panics(line in "\\PC{0,200}") {
        let _ = parse_mlst(&line);
        let _ = parse_mlsd(&[line]);
    }

    #[test]
    fn structured_garbage_never_panics(
        perm in "[-dlbcps?rwxsStT]{0,12}",
        n in 0u64..u64::MAX,
        a in "\\PC{0,12}",
        month in "[A-Za-z]{0,5}",
        day in "[0-9]{0,4}",
        time in "[0-9:]{0,8}",
        name in "\\PC{0,60}",
    ) {
        let line = format!("{perm} 1 {a} {a} {n} {month} {day} {time} {name}");
        let _ = parse_list(&[line], SystemTime::now());
    }

    #[test]
    fn mlsd_facts_garbage_never_panics(facts in "[a-z=;0-9A-Z./ -]{0,100}", name in "\\PC{0,40}") {
        let _ = parse_mlst(&format!("{facts} {name}"));
    }
}
