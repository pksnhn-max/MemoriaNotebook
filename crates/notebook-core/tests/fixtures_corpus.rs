//! Fixed corpus fixtures (corpus-a categories 1-9, corpus-a2
//! categories 10-16) for the two surfaces the brief assigns here:
//! source text into `chunk_text`, and script text into
//! `parse_script`.
//!
//! Where a fixture's shape is a matter of exact character counts
//! (380-char paragraphs and the like), the fixture is the generating
//! code below — a file full of 380 identical characters cannot be
//! reviewed and rots silently. Where the literal text is the point,
//! the fixture is a real file under tests/fixtures/.
//!
//! Every expectation is the implementation's face value, read from
//! the code before writing the test. Where the corpus design assumed
//! different behavior (numeric script speakers, one chunk per
//! paragraph, an "undefined" bucket), the record book lists the
//! discrepancy; these tests pin what the code actually does.

use notebook_core::chunk_text;
use notebook_core::voice::parse_script;

/// A paragraph of exactly `n` copies of `c`.
fn para(c: char, n: usize) -> String {
    std::iter::repeat_n(c, n).collect()
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

// ---------- chunk_text: empty and near-empty (categories 1, 3) ----------

#[test]
fn chunk_empty_inputs_yield_no_chunks() {
    assert!(chunk_text(include_str!("fixtures/text_00_empty.txt")).is_empty());
    assert!(chunk_text(include_str!("fixtures/text_02_all_empty.txt")).is_empty());
    assert!(chunk_text("").is_empty());
    assert!(chunk_text("   ").is_empty());
    assert!(chunk_text(" \n\n \n\n ").is_empty());
}

#[test]
fn chunk_single_line_is_one_chunk() {
    let chunks = chunk_text(include_str!("fixtures/text_01_single_line.txt"));
    assert_eq!(chunks, vec!["これは単一行のテスト入力です。改行文字は含みません。"]);
}

// ---------- chunk_text: blank lines around content (categories 4-6) ----------

#[test]
fn chunk_leading_blank_lines_are_skipped_and_short_paragraphs_pack() {
    // Two ~100-char paragraphs pack into one chunk joined by '\n':
    // the packer fills up to the 400-char target, it does not emit
    // one chunk per paragraph.
    let a = para('あ', 100);
    let b = para('い', 100);
    let text = format!("\n\n{a}\n\n{b}");
    let chunks = chunk_text(&text);
    assert_eq!(chunks, vec![format!("{a}\n{b}")]);
}

#[test]
fn chunk_middle_blank_lines_collapse_to_one_boundary() {
    let a = para('あ', 150);
    let b = para('い', 150);
    let c = para('う', 150);
    // Runs of blank lines between paragraphs; a+b pack (301 chars),
    // c would cross the target and starts a new chunk.
    let text = format!("{a}\n\n\n\n{b}\n\n{c}");
    let chunks = chunk_text(&text);
    assert_eq!(chunks, vec![format!("{a}\n{b}"), c]);
}

#[test]
fn chunk_trailing_blank_lines_make_no_empty_chunk() {
    let a = para('あ', 300);
    let b = para('い', 300);
    let text = format!("{a}\n\n{b}\n\n\n\n");
    let chunks = chunk_text(&text);
    assert_eq!(chunks, vec![a, b]);
}

#[test]
fn chunk_whitespace_only_paragraph_is_skipped() {
    let a = para('あ', 100);
    let b = para('い', 100);
    let text = format!("{a}\n\n   \n\n{b}");
    assert_eq!(chunk_text(&text), vec![format!("{a}\n{b}")]);
    let text = format!("{a}\n\n\t \t\n\n{b}");
    assert_eq!(chunk_text(&text), vec![format!("{a}\n{b}")]);
}

#[test]
fn chunk_whitespace_only_line_inside_a_paragraph_survives() {
    // Paragraphs split on "\n\n" only: a whitespace-only line joined
    // by single newlines is paragraph content, not a boundary.
    let text = "一行目です。\n   \n二行目です。";
    assert_eq!(chunk_text(text), vec![text.to_string()]);
}

// ---------- chunk_text: long input and hard split (category 8) ----------

#[test]
fn chunk_very_long_paragraph_hard_splits_at_the_target() {
    let text = para('あ', 10_000);
    let chunks = chunk_text(&text);
    assert_eq!(chunks.len(), 25);
    assert!(chunks.iter().all(|c| char_len(c) == 400));
    assert_eq!(chunks.concat(), text);
}

#[test]
fn chunk_hard_split_prefers_a_sentence_break_in_the_window() {
    // 851 chars: the only 。 in the first window sits at index 250,
    // inside the look-back range, so the first cut lands right after it.
    let text = format!("{}。{}", para('あ', 250), para('あ', 600));
    let chunks = chunk_text(&text);
    let sizes: Vec<usize> = chunks.iter().map(|c| char_len(c)).collect();
    assert_eq!(sizes, vec![251, 400, 200]);
    assert!(chunks[0].ends_with('。'));
    assert_eq!(chunks.concat(), text);
}

#[test]
fn chunk_hard_split_without_a_break_in_the_window_cuts_at_the_limit() {
    // 902 chars; the 。 at index 450 is outside the first window's
    // look-back range (200..400) and the one at 901 comes too late.
    let text = format!("{}。{}。", para('あ', 450), para('い', 450));
    let chunks = chunk_text(&text);
    let sizes: Vec<usize> = chunks.iter().map(|c| char_len(c)).collect();
    assert_eq!(sizes, vec![400, 400, 102]);
    assert_eq!(chunks.concat(), text);
}

// ---------- chunk_text: packing boundary (category 9) ----------

#[test]
fn chunk_packing_boundary_is_strictly_over_the_target() {
    // 200 + 1 (joining newline) + 199 = exactly 400: still one chunk.
    let a = para('あ', 200);
    let b = para('い', 199);
    let chunks = chunk_text(&format!("{a}\n\n{b}"));
    assert_eq!(chunks, vec![format!("{a}\n{b}")]);
    assert_eq!(char_len(&chunks[0]), 400);

    // 200 + 1 + 200 = 401 > 400: the second paragraph starts a chunk.
    let b = para('い', 200);
    let chunks = chunk_text(&format!("{a}\n\n{b}"));
    assert_eq!(chunks, vec![a, b]);
}

#[test]
fn chunk_boundary_crossing_paragraphs_stay_separate() {
    // The corpus design's 380/60/380 case: each following paragraph
    // would push the packed chunk over the target, so all three
    // stand alone.
    let a = para('あ', 380);
    let b = para('い', 60);
    let c = para('う', 380);
    let chunks = chunk_text(&format!("{a}\n\n{b}\n\n{c}"));
    assert_eq!(chunks, vec![a, b, c]);
}

// ---------- chunk_text: duplicates and trailing whitespace (categories 10, 11) ----------

#[test]
fn chunk_duplicate_paragraphs_are_kept_as_separate_chunks() {
    let p = para('あ', 380);
    let q = para('い', 100);
    let chunks = chunk_text(&format!("{p}\n\n{q}\n\n{p}"));
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0], p);
    assert_eq!(chunks[1], q);
    assert_eq!(chunks[0], chunks[2], "no deduplication");
}

#[test]
fn chunk_paragraphs_differing_only_in_trailing_whitespace_unify() {
    // Paragraphs are trimmed before packing, so the two copies of x
    // become the same text and pack together.
    let x = para('あ', 100);
    let text = format!("{x} \n\n{x}\t");
    let chunks = chunk_text(&text);
    assert_eq!(chunks, vec![format!("{x}\n{x}")]);
    assert!(chunks.iter().all(|c| !c.ends_with([' ', '\t'])));
}

// ---------- chunk_text: emoji and control characters (category 16) ----------

#[test]
fn chunk_counts_characters_not_bytes() {
    let text = "😀🎉🚀".repeat(30);
    assert_eq!(chunk_text(&text), vec![text.clone()]);
    // 900 emoji = 900 chars (3600 bytes): hard-split by character.
    let text = para('😀', 900);
    let chunks = chunk_text(&text);
    let sizes: Vec<usize> = chunks.iter().map(|c| char_len(c)).collect();
    assert_eq!(sizes, vec![400, 400, 100]);
    assert_eq!(chunks.concat(), text);
}

#[test]
fn chunk_control_characters_inside_a_paragraph_are_kept() {
    let text = "あ\u{0}い\u{1f}う";
    assert_eq!(chunk_text(text), vec![text.to_string()]);
}

// ---------- parse_script: empty and one-sided (categories 1, 2, 3) ----------

#[test]
fn script_empty_and_blank_only_are_errors() {
    // Face value: with no A and no B line the script is rejected;
    // the corpus design's "empty vector" does not happen.
    assert!(parse_script(include_str!("fixtures/text_00_empty.txt")).is_err());
    assert!(parse_script("\n\n\n").is_err());
    assert!(parse_script("   \n\t\n").is_err());
}

#[test]
fn script_single_speaker_only_is_an_error() {
    assert!(parse_script(include_str!("fixtures/script_single_a_only.txt")).is_err());
}

#[test]
fn script_numeric_speakers_are_not_script_lines() {
    // The corpus design wrote script fixtures with numeric speaker
    // prefixes ("0: ", "1: "). The implementation's speakers are the
    // letters A and B; numeric lines are skipped as unparseable, so
    // this input has no valid line at all and is rejected.
    assert!(parse_script(include_str!("fixtures/script_numeric_speakers.txt")).is_err());
}

// ---------- parse_script: blank lines (categories 4, 5, 6, 7) ----------

#[test]
fn script_blank_lines_are_skipped_anywhere() {
    let want = vec![
        (0u8, "最初の台詞です。".to_string()),
        (1u8, "2番目の台詞です。".to_string()),
    ];
    assert_eq!(
        parse_script(include_str!("fixtures/script_leading_empty.txt")).unwrap(),
        want
    );
    assert_eq!(
        parse_script(include_str!("fixtures/script_trailing_empty.txt")).unwrap(),
        want
    );
    // Whitespace-only lines are skipped the same way.
    let text = "A: 最初の台詞です。\n   \n\t\nB: 2番目の台詞です。\n";
    assert_eq!(parse_script(text).unwrap(), want);
}

#[test]
fn script_middle_blank_lines_do_not_break_the_sequence() {
    let got = parse_script(include_str!("fixtures/script_middle_empty.txt")).unwrap();
    assert_eq!(
        got,
        vec![
            (0u8, "最初の台詞です。".to_string()),
            (1u8, "2番目の台詞です。".to_string()),
            (0u8, "3番目の台詞です。".to_string()),
        ]
    );
}

// ---------- parse_script: duplicates, trailing whitespace (categories 10, 11) ----------

#[test]
fn script_duplicate_lines_are_kept_as_separate_entries() {
    let got = parse_script(include_str!("fixtures/script_duplicate_lines.txt")).unwrap();
    assert_eq!(got.len(), 3);
    assert_eq!(got[0], got[2], "no merging of identical lines");
    assert_eq!(got[0].1, "おはようございます。");
}

#[test]
fn script_line_content_is_trimmed() {
    let text = "A: 台詞です。   \nB: 次の台詞です。\t \n";
    let got = parse_script(text).unwrap();
    assert_eq!(
        got,
        vec![
            (0u8, "台詞です。".to_string()),
            (1u8, "次の台詞です。".to_string()),
        ]
    );
}

// ---------- parse_script: failing elements (categories 12-15) ----------

#[test]
fn script_unparseable_lines_are_skipped_wherever_they_sit() {
    let want = vec![
        (0u8, "最初の台詞です。".to_string()),
        (1u8, "2番目の台詞です。".to_string()),
    ];
    assert_eq!(
        parse_script(include_str!("fixtures/script_fail_first.txt")).unwrap(),
        want
    );
    assert_eq!(
        parse_script(include_str!("fixtures/script_fail_middle.txt")).unwrap(),
        want
    );
    assert_eq!(
        parse_script(include_str!("fixtures/script_fail_last.txt")).unwrap(),
        want
    );
}

#[test]
fn script_with_no_valid_line_at_all_is_an_error() {
    assert!(parse_script(include_str!("fixtures/script_fail_all.txt")).is_err());
}

// ---------- parse_script: empty content and colon forms (category 16) ----------

#[test]
fn script_lines_with_empty_content_do_not_count() {
    // "A: " with nothing after it is skipped, leaving no A line.
    assert!(parse_script(include_str!("fixtures/script_empty_content.txt")).is_err());
    // A later B line with content still counts.
    let text = "A: 実のある台詞です。\nB:\nB: 二行目の台詞です。";
    assert_eq!(
        parse_script(text).unwrap(),
        vec![
            (0u8, "実のある台詞です。".to_string()),
            (1u8, "二行目の台詞です。".to_string()),
        ]
    );
}

#[test]
fn script_fullwidth_colon_is_accepted() {
    let got = parse_script(include_str!("fixtures/script_fullwidth_colon.txt")).unwrap();
    assert_eq!(
        got,
        vec![
            (0u8, "最初の台詞です。".to_string()),
            (1u8, "2番目の台詞です。".to_string()),
        ]
    );
}

// ---------- parse_script: long input (category 8) ----------

#[test]
fn script_very_long_input_parses_every_line() {
    let mut text = String::new();
    for i in 0..100 {
        let who = if i % 2 == 0 { "A" } else { "B" };
        text.push_str(&format!("{who}: 台詞{i}です。{}\n", para('あ', 90)));
    }
    let got = parse_script(&text).unwrap();
    assert_eq!(got.len(), 100);
    assert!(got
        .iter()
        .enumerate()
        .all(|(i, (who, _))| *who == (i % 2) as u8));
    assert!(got[99].1.starts_with("台詞99です。"));
}
