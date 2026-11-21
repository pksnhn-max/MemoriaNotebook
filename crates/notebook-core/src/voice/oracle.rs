//! ORACLE — frozen verbatim copies of live functions from
//! `notebook-core/src/voice.rs`, for differential testing
//! (testing-manual §2, placement ①: a `#[cfg(test)]` oracle module;
//! this whole file is test-only because the `mod oracle;` declaration
//! in `voice.rs` is `#[cfg(test)]`).
//!
//! DO NOT EDIT the copied bodies. The oracle is frozen with its bugs;
//! if live and oracle disagree, suspect the live side (or the copy's
//! transcription — re-extract, never hand-fix).
//!
//! Provenance (copied 2026-11-21 from the live source at repo HEAD
//! 3118d042 by mechanical line-range extraction; see the W3 record book
//! `~/workspace/notebook-design/records/2026-11-21-w3-oracle-core.md`):
//! - `WavHeader`, `parse_wav_header`, `concat_wavs`, `load_consents`,
//!   `has_consent`, `record_consent`, `build_script_prompt`,
//!   `parse_script`, `wav_duration_sec`
//!   ← `crates/notebook-core/src/voice.rs`
//!
//! Differential property tests: `mod diff_tests` at the bottom.
//! Fixed-corpus fixture tests (tests/fixtures/wav): `mod fixture_tests`
//! at the bottom.

use std::path::Path;

#[derive(Debug, Clone, Copy)]
struct WavHeader {
    sample_rate: u32,
    num_channels: u16,
    bits_per_sample: u16,
    data_size: u32,
}

fn parse_wav_header(data: &[u8]) -> Result<(WavHeader, usize), String> {
    if data.len() < 44 {
        return Err("wav too short".to_string());
    }
    if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("not a valid RIFF/WAVE".to_string());
    }

    let mut offset = 12;
    let mut fmt_found = false;
    let mut data_offset = 0;
    let mut header = WavHeader {
        sample_rate: 0,
        num_channels: 0,
        bits_per_sample: 0,
        data_size: 0,
    };

    while offset + 8 <= data.len() {
        let chunk_id = &data[offset..offset + 4];
        let chunk_size = u32::from_le_bytes([
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]) as usize;

        if offset + 8 + chunk_size > data.len() {
            return Err("chunk exceeds file size".to_string());
        }

        if chunk_id == b"fmt " {
            if chunk_size < 16 {
                return Err("fmt chunk too small".to_string());
            }
            let fmt_data = &data[offset + 8..offset + 8 + chunk_size];
            let audio_format = u16::from_le_bytes([fmt_data[0], fmt_data[1]]);
            if audio_format != 1 {
                return Err("only PCM format supported".to_string());
            }
            header.num_channels = u16::from_le_bytes([fmt_data[2], fmt_data[3]]);
            header.sample_rate = u32::from_le_bytes([
                fmt_data[4], fmt_data[5], fmt_data[6], fmt_data[7],
            ]);
            header.bits_per_sample = u16::from_le_bytes([fmt_data[14], fmt_data[15]]);
            fmt_found = true;
        } else if chunk_id == b"data" {
            data_offset = offset + 8;
            header.data_size = chunk_size as u32;
            break;
        }

        offset += 8 + chunk_size;
        if chunk_size % 2 == 1 {
            offset += 1;
        }
    }

    if !fmt_found {
        return Err("fmt chunk not found".to_string());
    }
    if data_offset == 0 {
        return Err("data chunk not found".to_string());
    }

    Ok((header, data_offset))
}

pub fn concat_wavs(wavs: &[Vec<u8>], gap_ms: u32) -> Result<Vec<u8>, String> {
    if wavs.is_empty() {
        return Err("no wavs to concat".to_string());
    }

    let (first_header, _) = parse_wav_header(&wavs[0])?;
    let sample_rate = first_header.sample_rate;
    let num_channels = first_header.num_channels;
    let bits_per_sample = first_header.bits_per_sample;

    if sample_rate == 0 || num_channels == 0 || bits_per_sample == 0 {
        return Err("invalid wav format".to_string());
    }

    let mut all_pcm = Vec::new();
    let bytes_per_sample = (bits_per_sample / 8) as usize;
    let gap_samples = (sample_rate as u64 * gap_ms as u64 / 1000) as usize;
    let gap_bytes = gap_samples * num_channels as usize * bytes_per_sample;

    for (i, wav) in wavs.iter().enumerate() {
        let (header, data_offset) = parse_wav_header(wav)?;

        if header.sample_rate != sample_rate
            || header.num_channels != num_channels
            || header.bits_per_sample != bits_per_sample
        {
            return Err(format!("wav {} format mismatch", i));
        }

        let pcm_data = &wav[data_offset..data_offset + header.data_size as usize];
        all_pcm.extend_from_slice(pcm_data);

        if i + 1 < wavs.len() && gap_bytes > 0 {
            all_pcm.extend(std::iter::repeat(0u8).take(gap_bytes));
        }
    }

    let data_size = all_pcm.len() as u32;
    let riff_size = 36 + data_size;

    let mut out = Vec::with_capacity(44 + all_pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&num_channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    let byte_rate = sample_rate * num_channels as u32 * bytes_per_sample as u32;
    out.extend_from_slice(&byte_rate.to_le_bytes());
    let block_align = num_channels * bytes_per_sample as u16;
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_size.to_le_bytes());
    out.extend(all_pcm);

    Ok(out)
}

pub fn load_consents(data_dir: &Path) -> Vec<String> {
    let path = data_dir.join("voice_consents.json");
    if !path.exists() {
        return Vec::new();
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    match serde_json::from_str::<Vec<String>>(&content) {
        Ok(v) => v,
        Err(_) => Vec::new(),
    }
}

pub fn has_consent(data_dir: &Path, key: &str) -> bool {
    load_consents(data_dir).iter().any(|k| k == key)
}

pub fn record_consent(data_dir: &Path, key: &str) -> Result<(), String> {
    let mut consents = load_consents(data_dir);
    if consents.iter().any(|k| k == key) {
        return Ok(());
    }
    consents.push(key.to_string());
    let path = data_dir.join("voice_consents.json");
    let json = serde_json::to_string_pretty(&consents).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

pub fn build_script_prompt(nb_title: &str, source_text: &str, focus: Option<&str>) -> String {
    let focus_line = focus.map_or(String::new(), |f| format!("特に{}を中心に", f));
    format!(
        r#"あなたは音声番組の台本作家です。二人のホスト A（説明役）と B（聞き役）の対話台本を書いてください。

形式：
- 1行ごとに「A: 」または「B: 」で始め、交互に話す
- 全体で 1200〜2000 字程度
- 資料の内容だけを使い、資料に無いことは言わない
- 読みにくい漢字の語は自然な範囲でひらがなにする
- ト書き・見出し・引用符の装飾は付けない
{}
最後に資料本文を渡します。

資料タイトル: {}
資料本文:
{}"#,
        if focus_line.is_empty() { String::new() } else { format!("{}\n", focus_line) },
        nb_title,
        source_text
    )
}

pub fn parse_script(text: &str) -> Result<Vec<(u8, String)>, String> {
    let mut result = Vec::new();
    let mut has_a = false;
    let mut has_b = false;

    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let parsed = if let Some(c) = trimmed.strip_prefix("A:").or_else(|| trimmed.strip_prefix("A：")) {
            Some((0u8, c))
        } else if let Some(c) = trimmed.strip_prefix("B:").or_else(|| trimmed.strip_prefix("B：")) {
            Some((1u8, c))
        } else {
            None
        };
        if let Some((who, content)) = parsed {
            let content = content.trim();
            if content.is_empty() {
                continue;
            }
            if who == 0 { has_a = true; } else { has_b = true; }
            result.push((who, content.to_string()));
        }
    }

    if !has_a || !has_b {
        return Err("A と B の両方が最低1行ずつ必要です".to_string());
    }
    Ok(result)
}

pub fn wav_duration_sec(wav: &[u8]) -> Option<f64> {
    let (header, _) = parse_wav_header(wav).ok()?;
    let bytes_per_sample = (header.bits_per_sample / 8) as u32;
    let bytes_per_sec = header.sample_rate * (header.num_channels as u32) * bytes_per_sample;
    if bytes_per_sec == 0 {
        return None;
    }
    Some(header.data_size as f64 / bytes_per_sec as f64)
}

// =====================================================================
// Differential property tests: live vs the frozen oracle copies above.
// Every property runs at the manual's floor of 1,000 cases (§1).
// The oracle copies are the ones defined in this module (super::*);
// the live functions are the voice-module ones (crate::voice::*).
// =====================================================================
mod diff_tests {
    use super::{build_script_prompt as oracle_build_script_prompt,
                concat_wavs as oracle_concat_wavs, has_consent as oracle_has_consent,
                load_consents as oracle_load_consents, parse_script as oracle_parse_script,
                parse_wav_header as oracle_parse_wav_header,
                record_consent as oracle_record_consent,
                wav_duration_sec as oracle_wav_duration_sec, WavHeader as OracleWavHeader};
    use crate::voice::{build_script_prompt as live_build_script_prompt,
                       concat_wavs as live_concat_wavs, has_consent as live_has_consent,
                       load_consents as live_load_consents, parse_script as live_parse_script,
                       parse_wav_header as live_parse_wav_header,
                       record_consent as live_record_consent,
                       wav_duration_sec as live_wav_duration_sec, WavHeader as LiveWavHeader};
    use proptest::prelude::*;

    /// Project a parse result onto a comparable tuple (WavHeader has
    /// no PartialEq; the two sides' header types are distinct anyway).
    fn proj_live(
        r: Result<(LiveWavHeader, usize), String>,
    ) -> Result<(u32, u16, u16, u32, usize), String> {
        r.map(|(h, off)| (h.sample_rate, h.num_channels, h.bits_per_sample, h.data_size, off))
    }

    fn proj_oracle(
        r: Result<(OracleWavHeader, usize), String>,
    ) -> Result<(u32, u16, u16, u32, usize), String> {
        r.map(|(h, off)| (h.sample_rate, h.num_channels, h.bits_per_sample, h.data_size, off))
    }

    /// Canonical 44-byte-header WAV builder (same shape as the live
    /// code's own test builder, parameterized).
    fn build_wav(sample_rate: u32, channels: u16, bits: u16, data: &[u8]) -> Vec<u8> {
        let bytes_per_sample = (bits / 8) as u32;
        let byte_rate = sample_rate * channels as u32 * bytes_per_sample;
        let block_align = channels * bytes_per_sample as u16;
        let data_size = data.len() as u32;
        let mut w = Vec::with_capacity(44 + data.len());
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data_size).to_le_bytes());
        w.extend_from_slice(b"WAVE");
        w.extend_from_slice(b"fmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&channels.to_le_bytes());
        w.extend_from_slice(&sample_rate.to_le_bytes());
        w.extend_from_slice(&byte_rate.to_le_bytes());
        w.extend_from_slice(&block_align.to_le_bytes());
        w.extend_from_slice(&bits.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&data_size.to_le_bytes());
        w.extend_from_slice(data);
        w
    }

    fn apply_mutation(mut wav: Vec<u8>, kind: u8, pos: usize, byte: u8) -> Vec<u8> {
        match kind % 4 {
            0 => wav,
            1 => {
                let n = if wav.is_empty() { 0 } else { pos % (wav.len() + 1) };
                wav.truncate(n);
                wav
            }
            2 => {
                if !wav.is_empty() {
                    let i = pos % wav.len();
                    wav[i] = byte;
                }
                wav
            }
            _ => {
                if wav.len() >= 44 {
                    let i = 40 + (pos % 4);
                    wav[i] = byte;
                }
                wav
            }
        }
    }

    /// WAV-ish byte strings: canonical headers over varied formats and
    /// payloads, with truncations and byte corruptions mixed in.
    fn wav_strategy() -> impl Strategy<Value = Vec<u8>> {
        (
            prop_oneof![
                Just(24000u32),
                Just(48000),
                Just(8000),
                Just(44100),
                Just(0),
            ],
            prop_oneof![Just(1u16), Just(2)],
            prop_oneof![Just(16u16), Just(8), Just(24), Just(32), Just(0)],
            prop::collection::vec(any::<u8>(), 0..200),
            any::<u8>(),
            any::<usize>(),
            any::<u8>(),
        )
            .prop_map(|(sr, ch, bits, data, kind, pos, byte)| {
                apply_mutation(build_wav(sr, ch, bits, &data), kind, pos, byte)
            })
    }

    /// Concat/duration-safe variant of `wav_strategy`: the format
    /// fields (sample_rate / channels / bits) come only from the
    /// canonical sets, and mutations are limited to truncation, the
    /// data_size field, and payload bytes. Rationale (finding,
    /// 2026-11-21; see the W3 record book): with the free byte
    /// mutation, a corrupted num_channels / sample_rate feeds the
    /// multiplicative geometry of `concat_wavs` (gap silence =
    /// rate × gap × channels × bytes_per_sample; block_align is a
    /// u16 product), which made the generated property request
    /// ~10 GB of gap silence (allocator abort) and panic on u16
    /// overflow in the output header — on BOTH sides identically,
    /// so those cases carried no differential signal, only resource
    /// blow-ups. Header-field corruption stays covered by
    /// `parse_wav_header_matches_oracle` (full strategy), by the
    /// rejects fixtures, and by the pinned wild-geometry case in
    /// `fixture_tests`.
    fn concat_wav_strategy() -> impl Strategy<Value = Vec<u8>> {
        (
            prop_oneof![
                Just(24000u32),
                Just(48000),
                Just(8000),
                Just(44100),
                Just(0),
            ],
            prop_oneof![Just(1u16), Just(2)],
            prop_oneof![Just(16u16), Just(8), Just(24), Just(32), Just(0)],
            prop::collection::vec(any::<u8>(), 0..200),
            any::<u8>(),
            any::<usize>(),
            any::<u8>(),
        )
            .prop_map(|(sr, ch, bits, data, kind, pos, byte)| {
                apply_concat_mutation(build_wav(sr, ch, bits, &data), kind, pos, byte)
            })
    }

    fn apply_concat_mutation(mut wav: Vec<u8>, kind: u8, pos: usize, byte: u8) -> Vec<u8> {
        match kind % 4 {
            0 => wav,
            1 => {
                let n = if wav.is_empty() { 0 } else { pos % (wav.len() + 1) };
                wav.truncate(n);
                wav
            }
            2 => {
                if wav.len() >= 44 {
                    let i = 40 + (pos % 4);
                    wav[i] = byte;
                }
                wav
            }
            _ => {
                if wav.len() > 44 {
                    let i = 44 + (pos % (wav.len() - 44));
                    wav[i] = byte;
                }
                wav
            }
        }
    }

    fn piece_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                Just("あ"),
                Just("。"),
                Just("a"),
                Just("\n"),
                Just(" "),
            ],
            0..60,
        )
        .prop_map(|v| v.concat())
    }

    /// Script-shaped texts: A/B prefixes in both colon widths, stray
    /// leading spaces, unknown speakers, empty lines and content.
    fn script_strategy() -> impl Strategy<Value = String> {
        let prefix = prop_oneof![
            Just("A: "),
            Just("A："),
            Just("B: "),
            Just("B："),
            Just(" A: "),
            Just("B:"),
            Just("A:"),
            Just("C: "),
            Just(""),
            Just(" "),
        ];
        let content = prop::collection::vec(
            prop_oneof![
                Just("あ"),
                Just("い"),
                Just("。"),
                Just("a"),
                Just(" "),
                Just("："),
            ],
            0..20,
        )
        .prop_map(|v| v.concat());
        let line = (prefix, content).prop_map(|(p, c)| format!("{}{}", p, c));
        (prop::collection::vec(line, 0..12), any::<bool>()).prop_map(|(lines, trailing)| {
            let mut s = lines.join("\n");
            if trailing {
                s.push('\n');
            }
            s
        })
    }

    #[derive(Debug, Clone)]
    enum ConsentOp {
        Record(String),
        Has(String),
        Load,
    }

    fn consent_op_strategy() -> impl Strategy<Value = ConsentOp> {
        let key = prop::sample::select(vec!["zundamon", "metan", "zunko", "kiritan", "other"])
            .prop_map(|s: &'static str| s.to_string());
        prop_oneof![
            key.clone().prop_map(ConsentOp::Record),
            key.prop_map(ConsentOp::Has),
            Just(ConsentOp::Load),
        ]
    }

    fn initial_consents_strategy() -> impl Strategy<Value = Option<Vec<u8>>> {
        prop_oneof![
            Just(None),
            Just(Some(Vec::new())),
            Just(Some(b"not json".to_vec())),
            Just(Some(b"[\"zundamon\"]".to_vec())),
            prop::collection::vec(
                prop::sample::select(vec!["zundamon", "metan", "x"]),
                0..4,
            )
            .prop_map(|ks| Some(serde_json::to_string(&ks).unwrap().into_bytes())),
        ]
    }

    /// The geometry ranges live `parse_wav_header` accepts after
    /// the FA fix (2026-11-21): the parse boundary validates, the
    /// frozen oracle does not. Used to carve the intended
    /// difference out of the two properties below.
    fn geometry_in_range(sample_rate: u32, num_channels: u16, bits_per_sample: u16) -> bool {
        (1..=384_000).contains(&sample_rate)
            && (1..=256).contains(&num_channels)
            && matches!(bits_per_sample, 8 | 16 | 24 | 32)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn parse_wav_header_matches_oracle(wav in wav_strategy()) {
            let oracle = proj_oracle(oracle_parse_wav_header(&wav));
            let live = proj_live(live_parse_wav_header(&wav));
            match &oracle {
                // Intended difference (FA fix): the oracle accepts
                // this geometry, live rejects it at the parse
                // boundary. Yardstick item — see the FA record
                // book, 「物差し項目（要人間検め）」.
                Ok((rate, ch, bits, _, _)) if !geometry_in_range(*rate, *ch, *bits) => {
                    prop_assert!(live.is_err(),
                        "live must reject out-of-range geometry, got {live:?}");
                }
                _ => prop_assert_eq!(live, oracle),
            }
        }

        #[test]
        fn concat_wavs_matches_oracle(
            wavs in prop::collection::vec(concat_wav_strategy(), 0..3),
            gap in prop_oneof![Just(0u32), Just(1), Just(350), (0u32..5000)],
        ) {
            // Intended difference (FA fix): if any input's geometry
            // is outside the validated ranges, live rejects at
            // parse (an Err whose message differs from the oracle's
            // downstream checks); otherwise both sides must agree
            // exactly, bytes and error messages included.
            // Yardstick item — see the FA record book,
            // 「物差し項目（要人間検め）」.
            let any_out_of_range = wavs.iter().any(|w| {
                matches!(oracle_parse_wav_header(w),
                    Ok((h, _)) if !geometry_in_range(h.sample_rate, h.num_channels, h.bits_per_sample))
            });
            if any_out_of_range {
                prop_assert!(live_concat_wavs(&wavs, gap).is_err());
            } else {
                prop_assert_eq!(live_concat_wavs(&wavs, gap), oracle_concat_wavs(&wavs, gap));
            }
        }

        #[test]
        fn wav_duration_sec_matches_oracle(wav in concat_wav_strategy()) {
            prop_assert_eq!(live_wav_duration_sec(&wav), oracle_wav_duration_sec(&wav));
        }

        #[test]
        fn parse_script_matches_oracle(text in script_strategy()) {
            prop_assert_eq!(live_parse_script(&text), oracle_parse_script(&text));
        }

        #[test]
        fn build_script_prompt_matches_oracle(
            title in piece_strategy(),
            source in piece_strategy(),
            focus in prop::option::of(piece_strategy()),
        ) {
            prop_assert_eq!(
                live_build_script_prompt(&title, &source, focus.as_deref()),
                oracle_build_script_prompt(&title, &source, focus.as_deref())
            );
        }

        /// The consent trio is stateful (a JSON file in a data dir), so
        /// the property is an operation sequence applied to two fresh
        /// temp dirs — one per side — comparing every return value and
        /// the final file bytes.
        #[test]
        fn consents_match_oracle(
            ops in prop::collection::vec(consent_op_strategy(), 0..10),
            initial in initial_consents_strategy(),
            salt in any::<u64>(),
        ) {
            static COUNTER: std::sync::atomic::AtomicU64 =
                std::sync::atomic::AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let base = std::env::temp_dir().join(format!(
                "nb-oracle-consents-{}-{}-{}",
                std::process::id(),
                salt,
                n
            ));
            let dir_live = base.with_extension("live");
            let dir_oracle = base.with_extension("oracle");
            for d in [&dir_live, &dir_oracle] {
                std::fs::create_dir_all(d).unwrap();
                if let Some(bytes) = &initial {
                    std::fs::write(d.join("voice_consents.json"), bytes).unwrap();
                }
            }
            for op in &ops {
                match op {
                    ConsentOp::Record(k) => prop_assert_eq!(
                        live_record_consent(&dir_live, k),
                        oracle_record_consent(&dir_oracle, k)
                    ),
                    ConsentOp::Has(k) => prop_assert_eq!(
                        live_has_consent(&dir_live, k),
                        oracle_has_consent(&dir_oracle, k)
                    ),
                    ConsentOp::Load => prop_assert_eq!(
                        live_load_consents(&dir_live),
                        oracle_load_consents(&dir_oracle)
                    ),
                }
            }
            prop_assert_eq!(
                live_load_consents(&dir_live),
                oracle_load_consents(&dir_oracle)
            );
            let bytes_live = std::fs::read(dir_live.join("voice_consents.json")).ok();
            let bytes_oracle = std::fs::read(dir_oracle.join("voice_consents.json")).ok();
            prop_assert_eq!(bytes_live, bytes_oracle);
            let _ = std::fs::remove_dir_all(&dir_live);
            let _ = std::fs::remove_dir_all(&dir_oracle);
        }
    }
}

// =====================================================================
// Fixed-corpus fixture tests for the WAV surface (tests/fixtures/wav).
// Shapes come from the corpus-b2 annex. For defined cases the test
// asserts live == oracle (byte equality, manual §2) plus structural
// facts read off the output with an independent fixed-offset parser.
// Inputs the annex puts in the undefined bucket (rejects/) get no
// fixed expectation: the test only asserts neither side panics, and
// the current face value is recorded in comments / the fixtures
// README. Note two annex expectations do NOT hold against the real
// code and are therefore not asserted: an empty input list is an
// error (not a 44-byte empty WAV), and a gap is inserted only between
// WAVs (a single WAV gains no trailing silence).
// =====================================================================
mod fixture_tests {
    use super::{concat_wavs as oracle_concat_wavs, wav_duration_sec as oracle_wav_duration_sec};
    use crate::voice::{concat_wavs as live_concat_wavs, wav_duration_sec as live_wav_duration_sec};
    use std::path::PathBuf;

    fn fixture_path(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(rel)
    }

    fn read_fixture(rel: &str) -> Vec<u8> {
        std::fs::read(fixture_path(rel))
            .unwrap_or_else(|e| panic!("fixture {rel} unreadable: {e}"))
    }

    /// Independent structural read of a canonical 44-byte-header WAV:
    /// (sample_rate, channels, bits_per_sample, data_size) taken from
    /// the fixed header offsets. Only valid for canonical files (all
    /// generated inputs and all concat outputs are canonical).
    fn canonical_info(bytes: &[u8]) -> (u32, u16, u16, u32) {
        assert!(bytes.len() >= 44, "not a canonical wav ({} bytes)", bytes.len());
        let rate = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
        let channels = u16::from_le_bytes(bytes[22..24].try_into().unwrap());
        let bits = u16::from_le_bytes(bytes[34..36].try_into().unwrap());
        let data_size = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
        (rate, channels, bits, data_size)
    }

    fn frames_of(bytes: &[u8]) -> u32 {
        let (rate, ch, bits, data_size) = canonical_info(bytes);
        assert_eq!(rate, 24000);
        data_size / (ch as u32 * (bits / 8) as u32)
    }

    /// Run one defined concat case: live must equal oracle byte for
    /// byte, the output must hold the expected frame count, and both
    /// durations must equal frames/rate.
    fn assert_concat_case(files: &[&str], gap_ms: u32, expected_frames: u32) {
        let wavs: Vec<Vec<u8>> = files.iter().map(|f| read_fixture(f)).collect();
        let live = live_concat_wavs(&wavs, gap_ms);
        let oracle = oracle_concat_wavs(&wavs, gap_ms);
        assert_eq!(live, oracle, "live/oracle disagree for {files:?} gap {gap_ms}");
        let out = live.unwrap_or_else(|e| panic!("concat of {files:?} failed: {e}"));
        assert_eq!(frames_of(&out), expected_frames, "frame count for {files:?}");
        let expected_dur = expected_frames as f64 / 24000.0;
        assert_eq!(live_wav_duration_sec(&out), Some(expected_dur));
        assert_eq!(oracle_wav_duration_sec(&out), Some(expected_dur));
    }

    #[test]
    fn wav_fixtures_defined_cases() {
        // Empty list: defined by the real code as an error on both sides.
        assert_eq!(live_concat_wavs(&[], 0), oracle_concat_wavs(&[], 0));
        assert!(live_concat_wavs(&[], 0).is_err());

        // Single WAV, no gap: passes through (canonical rebuild).
        assert_concat_case(&["wav/single_100ms.wav"], 0, 2400);
        // Single WAV with a gap: the gap goes only between WAVs, so
        // nothing is appended (the annex's trailing-gap expectation
        // does not hold against the real code).
        assert_concat_case(&["wav/single_100ms.wav"], 50, 2400);
        // Two WAVs, gap 0 and gap 350 (the pre-existing unit-test shape).
        assert_concat_case(&["wav/silence_100ms.wav", "wav/silence_100ms.wav"], 0, 4800);
        assert_concat_case(
            &["wav/silence_100ms.wav", "wav/silence_200ms.wav"],
            350,
            2400 + 8400 + 4800,
        );
        // Zero-length data chunk(s).
        assert_concat_case(&["wav/zero_data.wav"], 0, 0);
        assert_concat_case(&["wav/zero_data.wav", "wav/zero_data.wav"], 100, 2400);
        // Tail-silence shapes: single passes through; a pair joins
        // directly with no silence inserted at gap 0.
        assert_concat_case(&["wav/tail_sil_100ms.wav"], 100, 14400);
        assert_concat_case(
            &["wav/tail_sil_100ms.wav", "wav/tail_sil_100ms.wav"],
            0,
            28800,
        );
        // Duplicate short WAVs.
        assert_concat_case(
            &["wav/silence_50ms.wav", "wav/silence_50ms.wav", "wav/silence_50ms.wav"],
            0,
            3600,
        );
        // Large gap (10 s) between two short WAVs.
        assert_concat_case(
            &["wav/silence_100ms.wav", "wav/silence_100ms.wav"],
            10000,
            2400 + 240000 + 2400,
        );
        // Long input (10 minutes of silence, ~28 MB fixture).
        assert_concat_case(&["wav/long_10min.wav"], 0, 14_400_000);
    }

    #[test]
    fn wav_fixtures_duration_cases() {
        for (rel, frames) in [
            ("wav/silence_100ms.wav", 2400u32),
            ("wav/zero_data.wav", 0),
            ("wav/tail_sil_100ms.wav", 14400),
            ("wav/tail_sil_200ms.wav", 16800),
            ("wav/tail_sil_300ms.wav", 19200),
            ("wav/long_10min.wav", 14_400_000),
        ] {
            let wav = read_fixture(rel);
            let expected = frames as f64 / 24000.0;
            assert_eq!(live_wav_duration_sec(&wav), Some(expected), "{rel}");
            assert_eq!(oracle_wav_duration_sec(&wav), Some(expected), "{rel}");
        }
    }

    /// Undefined bucket (annex categories 3, 5, 8, 11): no fixed
    /// expectation; the only assertion is that neither side panics.
    /// Current face values, read off the live source: bad RIFF/WAVE
    /// ids → Err("not a valid RIFF/WAVE"); the 5-byte text file →
    /// Err("wav too short"); header_only → Err("data chunk not
    /// found"); oversized/truncated data → Err("chunk exceeds file
    /// size"); mixed formats → Err("wav {i} format mismatch");
    /// bits32float (format tag 3) → Err("only PCM format supported");
    /// bad_fmt_size (fmt chunk of 18 bytes) is actually accepted.
    #[test]
    fn wav_fixtures_undefined_bucket_does_not_panic() {
        let good = read_fixture("wav/good_100ms.wav");
        let reject_names = [
            "wav/rejects/header_only.wav",
            "wav/rejects/sr_48k.wav",
            "wav/rejects/sr_8k.wav",
            "wav/rejects/ch2.wav",
            "wav/rejects/bits24.wav",
            "wav/rejects/bits32float.wav",
            "wav/rejects/bad_riff_id.wav",
            "wav/rejects/bad_wave_id.wav",
            "wav/rejects/bad_fmt_size.wav",
            "wav/rejects/bad_data_size.wav",
            "wav/rejects/truncated.wav",
            "wav/rejects/not_riff.mp3",
            "wav/rejects/not_riff.raw",
            "wav/rejects/not_riff.txt",
        ];
        for rel in reject_names {
            let bad = read_fixture(rel);
            let _ = live_wav_duration_sec(&bad);
            let _ = oracle_wav_duration_sec(&bad);
            let _ = live_concat_wavs(&[bad.clone()], 0);
            let _ = oracle_concat_wavs(&[bad.clone()], 0);
            // Failing element at head / middle / tail / everywhere.
            let _ = live_concat_wavs(&[bad.clone(), good.clone(), good.clone()], 10);
            let _ = live_concat_wavs(&[good.clone(), bad.clone(), good.clone()], 10);
            let _ = live_concat_wavs(&[good.clone(), good.clone(), bad.clone()], 10);
            let _ = live_concat_wavs(&[bad.clone(), bad.clone(), bad.clone()], 10);
            let _ = oracle_concat_wavs(&[bad.clone(), good.clone(), good.clone()], 10);
            let _ = oracle_concat_wavs(&[good.clone(), bad.clone(), good.clone()], 10);
            let _ = oracle_concat_wavs(&[good.clone(), good.clone(), bad.clone()], 10);
            let _ = oracle_concat_wavs(&[bad.clone(), bad.clone(), bad], 10);
        }
        // Mixed-format pairs (sample rate / channels / bit depth).
        for other in [
            "wav/rejects/sr_48k.wav",
            "wav/rejects/sr_8k.wav",
            "wav/rejects/ch2.wav",
            "wav/rejects/bits24.wav",
            "wav/rejects/bits32float.wav",
        ] {
            let other = read_fixture(other);
            let _ = live_concat_wavs(&[good.clone(), other.clone()], 0);
            let _ = oracle_concat_wavs(&[good.clone(), other], 0);
        }
    }

    /// Intended difference, FA fix (2026-11-21) — yardstick item,
    /// proposed here and recorded for human inspection in the FA
    /// record book, 「物差し項目（要人間検め）」.
    ///
    /// The header below (pinned from proptest regression seed
    /// cc 735c222e08f99009ccce6eb46618b328ab1b8917149f7772b6ac9bdc44506010)
    /// declares num_channels = 22017 and 24 bits/sample.
    /// Old agreed behavior (the frozen oracle, verbatim): the
    /// output-header arithmetic of `concat_wavs` overflows —
    /// block_align is the u16 product 22017 × 3 = 66051 — so debug
    /// builds panic and release builds wrap and emit bytes;
    /// duration returns Some(64 / (24000 × 22017 × 3)).
    /// New live behavior: `parse_wav_header` validates geometry at
    /// the parse boundary (num_channels must be 1..=256), so
    /// concat is a clean Err and duration is None — no panic.
    #[test]
    fn wild_format_geometry_live_rejects_intended_difference() {
        let mut wav: Vec<u8> = vec![
            82, 73, 70, 70, 100, 0, 0, 0, 87, 65, 86, 69, 102, 109, 116, 32, 16, 0, 0, 0, 1, 0,
            1, 86, 192, 93, 0, 0, 64, 25, 1, 0, 3, 0, 24, 0, 100, 97, 116, 97, 64, 0, 0, 0,
        ];
        wav.extend(std::iter::repeat(0u8).take(64));
        // Live: rejected at the parse boundary, no panic.
        assert!(live_concat_wavs(&[wav.clone()], 0).is_err());
        assert_eq!(live_wav_duration_sec(&wav), None);
        // Oracle (frozen old code): never returns a clean Err on
        // this input — it panics in debug builds and wraps through
        // to Ok(bytes) in release builds; duration still computes.
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            oracle_concat_wavs(&[wav.clone()], 0)
        })) {
            Ok(Ok(_)) => {} // release: wrapped arithmetic, bytes out
            Ok(Err(e)) => panic!("oracle unexpectedly returned a clean Err: {e}"),
            Err(_) => {} // debug: arithmetic-overflow panic
        }
        assert_eq!(oracle_wav_duration_sec(&wav), Some(64.0 / 1_585_224_000.0));
    }
}
