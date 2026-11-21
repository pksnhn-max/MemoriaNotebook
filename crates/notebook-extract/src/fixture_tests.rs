//! Fixture tests (testing-manual §2, fixed layer): every file in
//! `tests/fixtures/` (see its README for provenance and the
//! regeneration procedure) is fed to both the live implementation
//! and the frozen oracle; the outcomes must be identical. These
//! tests run on every suite run — the fixed layer is the part the
//! random generators are not trusted to reach every time.

use super::oracle;
use super::{extract_epub, extract_txt, html_to_text, ExtractError};

fn fixture(rel: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture {path}: {e}"))
}

fn outcome(r: Result<String, ExtractError>) -> (bool, String) {
    match r {
        Ok(s) => (true, s),
        Err(e) => (false, format!("{e}")),
    }
}

const TXT_FIXTURES: &[&str] = &[
    "00_empty.bin",
    "01_single_line_utf8.bin",
    "01_single_line_sjis.bin",
    "02_all_empty_utf8.bin",
    "02_all_empty_sjis.bin",
    "03_leading_empty_utf8.bin",
    "03_leading_empty_sjis.bin",
    "04_middle_empty_utf8.bin",
    "04_middle_empty_sjis.bin",
    "05_trailing_empty_utf8.bin",
    "05_trailing_empty_sjis.bin",
    "06_whitespace_only_utf8.bin",
    "06_whitespace_only_sjis.bin",
    "07_very_long_utf8.bin",
    "07_very_long_sjis.bin",
    "08_boundary_cross_utf8.bin",
    "08_boundary_cross_sjis.bin",
    "10_dup_lines.bin",
    "12_fail_first.bin",
    "13_fail_middle.bin",
    "14_fail_last.bin",
    "15_fail_all.bin",
];

const HTML_FIXTURES: &[&str] = &[
    "00_empty.html",
    "01_single_line.html",
    "02_all_empty.html",
    "03_leading_empty.html",
    "04_middle_empty.html",
    "05_trailing_empty.html",
    "06_whitespace_only.html",
    "07_very_long.html",
    "08_boundary_cross.html",
    "10_dup_lines.html",
    "12_fail_first.html",
    "13_fail_middle.html",
    "14_fail_last.html",
    "15_fail_all.html",
];

const EPUB_FIXTURES: &[&str] = &[
    "00_empty.epub",
    "01_single_line.epub",
    "02_all_empty.epub",
    "03_leading_empty.epub",
    "04_middle_empty.epub",
    "05_trailing_empty.epub",
    "06_whitespace_only.epub",
    "07_very_long.epub",
    "08_boundary_cross.epub",
    "10_dup_lines.epub",
    "12_fail_first.epub",
    "13_fail_middle.epub",
    "14_fail_last.epub",
    "15_fail_all.epub",
];

#[test]
fn fixtures_txt_live_matches_oracle() {
    for name in TXT_FIXTURES {
        let bytes = fixture(&format!("txt/{name}"));
        assert_eq!(
            extract_txt(&bytes),
            oracle::extract_txt(&bytes),
            "txt fixture {name}"
        );
    }
}

#[test]
fn fixtures_html_live_matches_oracle() {
    for name in HTML_FIXTURES {
        let bytes = fixture(&format!("html/{name}"));
        let html = String::from_utf8(bytes).unwrap();
        assert_eq!(
            html_to_text(&html),
            oracle::html_to_text(&html),
            "html fixture {name}"
        );
    }
}

#[test]
fn fixtures_epub_live_matches_oracle() {
    for name in EPUB_FIXTURES {
        let bytes = fixture(&format!("epub/{name}"));
        assert_eq!(
            outcome(extract_epub(&bytes)),
            outcome(oracle::extract_epub(&bytes)),
            "epub fixture {name}"
        );
    }
}

/// Category 16 — the undefined-verdict bucket (testing-manual §2):
/// pinned differentially like everything else, but no correctness
/// claim attaches to the outcomes themselves.
#[test]
fn fixtures_undefined_bucket_live_matches_oracle() {
    let bytes = fixture("undefined/txt_empty.bin");
    assert_eq!(extract_txt(&bytes), oracle::extract_txt(&bytes));

    let bytes = fixture("undefined/html_no_visible.html");
    let html = String::from_utf8(bytes).unwrap();
    assert_eq!(html_to_text(&html), oracle::html_to_text(&html));

    let bytes = fixture("undefined/epub_nav_only.epub");
    assert_eq!(
        outcome(extract_epub(&bytes)),
        outcome(oracle::extract_epub(&bytes))
    );
}
