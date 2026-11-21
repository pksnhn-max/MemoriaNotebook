//! 性質テスト：音声の純部（annex `props-voice.md` §1〜§4 の全候補）。
//!
//! - §1 `parse_script`：往復・不発・形式整合。
//! - §2 `build_script_prompt`：形式整合・非干渉。
//! - §3 `concat_wavs` / `wav_duration_sec`：形式整合（独立パーサ）・等価・往復・不発。
//!   WAV の読み手は WAV 仕様から独立に書いたパーサで、実装の `parse_wav_header`
//!   は流用しない（brief の裁定）。
//! - §4 `VOICE_CATALOG`：列挙で尽きる物は全件走査、不在キーの等価だけ生成で回す。
//!
//! ケース数の床は生成する性質すべて 1,000（brief の裁定。annex の 200／500 行も引き上げ）。
//!
//! 旧・既知の不成立（記録帳 W2 所見1）：ヘッダとして正当でも sample_rate が極端な
//! WAV では `wav_duration_sec` と `concat_wavs` が u32 乗算の溢れで panic した。
//! FA 修正（2026-11-21）で parse の境界検証が入り解消。反例は
//! `extreme_sample_rate_does_not_panic` として回帰テスト化して常時走らせる。
//! 生成性質のレート域は実在域（≤192,000）のまま。

use notebook_core::voice::{
    build_script_prompt, concat_wavs, parse_script, voice_by_key, wav_duration_sec, VOICE_CATALOG,
    VVM_BASE_URL,
};
use proptest::prelude::*;
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// §3 用：独立した WAV 構築器と読み手（WAV 仕様から書いた。実装と共有しない）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct WavSpec {
    rate: u32,
    channels: u16,
    bits: u16,
    pcm: Vec<u8>,
    /// fmt チャンクの 16 バイトを超える分（0／2／24 を使う）。
    fmt_extra: usize,
    /// fmt と data の間に入れる余分なチャンク（id は4文字）。
    extra_chunks: Vec<(String, Vec<u8>)>,
    /// data を fmt より前に置く（読み手は拒否すべき形）。
    data_first: bool,
}

fn block_align(channels: u16, bits: u16) -> usize {
    channels as usize * (bits / 8) as usize
}

fn gap_bytes(rate: u32, channels: u16, bits: u16, gap_ms: u32) -> usize {
    let samples = (rate as u64 * gap_ms as u64 / 1000) as usize;
    samples * block_align(channels, bits)
}

fn build_wav(spec: &WavSpec) -> Vec<u8> {
    let bytes_per_sample = (spec.bits / 8) as u32;
    let byte_rate = spec
        .rate
        .wrapping_mul(spec.channels as u32)
        .wrapping_mul(bytes_per_sample);
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes()); // PCM
    fmt.extend_from_slice(&spec.channels.to_le_bytes());
    fmt.extend_from_slice(&spec.rate.to_le_bytes());
    fmt.extend_from_slice(&byte_rate.to_le_bytes());
    fmt.extend_from_slice(&(spec.channels * bytes_per_sample as u16).to_le_bytes());
    fmt.extend_from_slice(&spec.bits.to_le_bytes());
    fmt.extend(std::iter::repeat(0u8).take(spec.fmt_extra));

    fn push_chunk(chunks: &mut Vec<u8>, id: &str, payload: &[u8]) {
        chunks.extend_from_slice(id.as_bytes());
        chunks.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        chunks.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            chunks.push(0); // チャンクは偶数境界に詰める
        }
    }

    let mut chunks = Vec::new();
    if spec.data_first {
        push_chunk(&mut chunks, "data", &spec.pcm);
        push_chunk(&mut chunks, "fmt ", &fmt);
    } else {
        push_chunk(&mut chunks, "fmt ", &fmt);
        for (id, payload) in &spec.extra_chunks {
            push_chunk(&mut chunks, id, payload);
        }
        push_chunk(&mut chunks, "data", &spec.pcm);
    }

    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(4 + chunks.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(&chunks);
    out
}

#[derive(Debug, PartialEq)]
struct ParsedWav {
    rate: u32,
    channels: u16,
    bits: u16,
    audio_format: u16,
    data: Vec<u8>,
}

/// WAV 仕様から書いた独立パーサ。RIFF サイズ欄と実長の一致まで検める。
fn parse_wav_independent(bytes: &[u8]) -> Option<ParsedWav> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let riff_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    if riff_size + 8 != bytes.len() {
        return None;
    }
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut data: Option<Vec<u8>> = None;
    let mut off = 12usize;
    while off + 8 <= bytes.len() {
        let id = &bytes[off..off + 4];
        let size = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap()) as usize;
        let start = off + 8;
        if start + size > bytes.len() {
            return None;
        }
        let payload = &bytes[start..start + size];
        if id == b"fmt " {
            if size < 16 {
                return None;
            }
            fmt = Some((
                u16::from_le_bytes(payload[0..2].try_into().unwrap()),
                u16::from_le_bytes(payload[2..4].try_into().unwrap()),
                u32::from_le_bytes(payload[4..8].try_into().unwrap()),
                u16::from_le_bytes(payload[14..16].try_into().unwrap()),
            ));
        } else if id == b"data" {
            data = Some(payload.to_vec());
        }
        off = start + size + (size % 2);
    }
    let (audio_format, channels, rate, bits) = fmt?;
    Some(ParsedWav {
        rate,
        channels,
        bits,
        audio_format,
        data: data?,
    })
}

// ---------------------------------------------------------------------------
// §3 の生成器
// ---------------------------------------------------------------------------

fn format_strategy() -> impl Strategy<Value = (u32, u16, u16)> {
    (
        prop::sample::select(vec![
            8000u32, 16000, 22050, 24000, 44100, 48000, 96000, 192000,
        ]),
        prop::sample::select(vec![1u16, 2]),
        prop::sample::select(vec![8u16, 16, 24, 32]),
    )
}

fn wav_spec_strategy(fmt: (u32, u16, u16)) -> impl Strategy<Value = WavSpec> {
    let (rate, channels, bits) = fmt;
    let align = block_align(channels, bits);
    (
        0..=2000usize, // サンプル数
        prop::sample::select(vec![0usize, 0, 0, 2, 24]),
        prop::collection::vec(
            (
                prop::sample::select(vec!["LIST", "JUNK", "bext"]),
                prop::collection::vec(any::<u8>(), 0..=20),
            ),
            0..=2,
        ),
    )
        .prop_flat_map(move |(samples, fmt_extra, extras)| {
            prop::collection::vec(any::<u8>(), samples * align..=samples * align).prop_map(
                move |pcm| WavSpec {
                    rate,
                    channels,
                    bits,
                    pcm,
                    fmt_extra,
                    extra_chunks: extras
                        .iter()
                        .map(|(id, p)| (id.to_string(), p.clone()))
                        .collect(),
                    data_first: false,
                },
            )
        })
}

/// (同形式の WAV バイト列の組, gap_ms, 形式) — 連結が成功すべきケース。
fn valid_concat_case() -> impl Strategy<Value = (Vec<Vec<u8>>, u32, (u32, u16, u16))> {
    format_strategy().prop_flat_map(|fmt| {
        let specs = prop::collection::vec(wav_spec_strategy(fmt), 1..=4);
        (specs, 0..=2000u32).prop_map(move |(specs, gap)| {
            (specs.iter().map(build_wav).collect(), gap, fmt)
        })
    })
}

fn byte_rate(fmt: (u32, u16, u16)) -> f64 {
    let (rate, channels, bits) = fmt;
    (rate as u64 * channels as u64 * (bits / 8) as u64) as f64
}

// ---------------------------------------------------------------------------
// §1 parse_script
// ---------------------------------------------------------------------------

/// 台本の1行（生成側）。content は改行を含まない任意文字列。
fn script_line_strategy() -> impl Strategy<Value = (bool, String)> {
    (
        any::<bool>(),
        prop::collection::vec(
            prop::sample::select(vec![
                'あ', 'い', 'う', 'A', 'B', ':', '：', ' ', '　', 'x', '1', '。', '\t',
            ]),
            0..20,
        )
        .prop_map(|cs| cs.into_iter().collect::<String>()),
    )
}

/// (台本テキスト, 期待される (話者, 中身) 列)。パースが Ok になる構成で組む：
/// 中身が空でない行だけが残り、A・B 両方が最低1行は残るよう補う。
fn valid_script_strategy() -> impl Strategy<Value = (String, Vec<(u8, String)>)> {
    (
        prop::collection::vec(script_line_strategy(), 1..=12),
        prop::collection::vec(any::<bool>(), 0..4), // 空行を挟む位置の気分
    )
        .prop_map(|(lines, _)| {
            let mut expected: Vec<(u8, String)> = Vec::new();
            let mut text_lines: Vec<String> = Vec::new();
            for (is_b, raw) in &lines {
                let content = raw.trim().to_string();
                let (prefix, who) = if *is_b { ("B", 1u8) } else { ("A", 0u8) };
                let colon = if (raw.len() + who as usize) % 2 == 0 {
                    ":"
                } else {
                    "："
                };
                text_lines.push(format!("{prefix}{colon} {raw}"));
                if !content.is_empty() {
                    expected.push((who, content));
                }
            }
            if !expected.iter().any(|(w, _)| *w == 0) {
                text_lines.push("A: あ".to_string());
                expected.push((0, "あ".to_string()));
            }
            if !expected.iter().any(|(w, _)| *w == 1) {
                text_lines.push("B: い".to_string());
                expected.push((1, "い".to_string()));
            }
            (text_lines.join("\n"), expected)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// 1.1 parse_script の往復：(話者, 中身) 列を正規形に直してパースすると元に戻り、
    /// その直し直し（再直列化）も文字列として安定する。
    #[test]
    fn script_roundtrip(case in valid_script_strategy()) {
        let (_text, expected) = case;
        let canonical = expected
            .iter()
            .map(|(w, c)| format!("{}: {}", if *w == 0 { "A" } else { "B" }, c))
            .collect::<Vec<_>>()
            .join("\n");
        let parsed = parse_script(&canonical);
        prop_assert_eq!(parsed.as_ref().map(|v| v.as_slice()), Ok(expected.as_slice()));
        let reparsed_text = parsed
            .unwrap()
            .iter()
            .map(|(w, c)| format!("{}: {}", if *w == 0 { "A" } else { "B" }, c))
            .collect::<Vec<_>>()
            .join("\n");
        prop_assert_eq!(reparsed_text, canonical);
    }

    /// 1.3 parse_script の形式整合＋等価：構成した台本は期待列その物にパースされ、
    /// 期待列は「話者は 0/1・中身は非空・両話者を含む・順序一致」を満たす。
    #[test]
    fn script_parses_to_expected(case in valid_script_strategy()) {
        let (text, expected) = case;
        let got = parse_script(&text);
        prop_assert_eq!(got.as_ref().map(|v| v.as_slice()), Ok(expected.as_slice()));
        for (who, content) in &expected {
            prop_assert!(*who == 0 || *who == 1);
            prop_assert!(!content.is_empty());
        }
        prop_assert!(expected.iter().any(|(w, _)| *w == 0));
        prop_assert!(expected.iter().any(|(w, _)| *w == 1));
    }

    /// 1.3（任意入力版）：どんな文字列でも、Ok なら戻り値は不変条件を満たす。
    #[test]
    fn script_ok_implies_wellformed(s in any::<String>()) {
        if let Ok(v) = parse_script(&s) {
            prop_assert!(v.iter().all(|(w, _)| *w == 0 || *w == 1));
            prop_assert!(v.iter().all(|(_, c)| !c.is_empty()));
            prop_assert!(v.iter().any(|(w, _)| *w == 0));
            prop_assert!(v.iter().any(|(w, _)| *w == 1));
        }
    }

    /// 1.2 parse_script の不発：話者の欠けた台本・中身の無い台本は確実に Err。
    /// blanks は空白文字だけの列（中身が空になる行を作るため）。
    #[test]
    fn script_rejects_incomplete(
        family in 0..6u8,
        lines in prop::collection::vec("[^\n]{1,20}", 1..=5),
        blanks in prop::collection::vec(
            prop::collection::vec(prop::sample::select(vec![' ', '\t', '　']), 0..=3)
                .prop_map(|cs| cs.into_iter().collect::<String>()),
            1..=5,
        ),
    ) {
        let text = match family {
            0 => "  \n\t\n　".to_string(),
            1 => lines.iter().map(|l| format!("A: {l}")).collect::<Vec<_>>().join("\n"),
            2 => lines.iter().map(|l| format!("B: {l}")).collect::<Vec<_>>().join("\n"),
            3 => blanks
                .iter()
                .enumerate()
                .map(|(i, b)| format!("{}:{b}", if i % 2 == 0 { "A" } else { "B" }))
                .collect::<Vec<_>>()
                .join("\n"),
            4 => lines
                .iter()
                .map(|l| {
                    // 前缀なしを保証する：trim 後に A:/B: で始まる行は頭に x を足す
                    let t = l.trim_start();
                    if t.starts_with("A:") || t.starts_with("A：") || t.starts_with("B:") || t.starts_with("B：") {
                        format!("x{l}")
                    } else {
                        l.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
            _ => {
                // A は中身あり、B は中身なし（B 側が欠けるので Err）
                let mut v: Vec<String> = lines.iter().map(|l| format!("A: {l}")).collect();
                v.extend(blanks.iter().map(|b| format!("B:{b}")));
                v.join("\n")
            }
        };
        let r = parse_script(&text);
        prop_assert!(r.is_err(), "family {} must be Err, got {:?} for {:?}", family, r, text);
    }
}

// ---------------------------------------------------------------------------
// §2 build_script_prompt
// ---------------------------------------------------------------------------

/// 「特に」を含まない文字列（focus=None の不在assert 用）。
fn marker_free_string() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec![
            'あ', 'い', 'う', 'え', 'お', 'a', 'b', '1', ' ', '\n', '。', '題', '本', '文',
        ]),
        0..30,
    )
    .prop_map(|cs| cs.into_iter().collect())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// 2.1 build_script_prompt の形式整合：題・本文がそのまま入り、
    /// focus があればその行が入り、A:/B: の形式指示を含む。
    #[test]
    fn prompt_contains_parts_verbatim(
        title in any::<String>(),
        source in any::<String>(),
        focus in prop::option::of(any::<String>()),
    ) {
        let p = build_script_prompt(&title, &source, focus.as_deref());
        prop_assert!(p.contains(&format!("資料タイトル: {title}")), "title missing");
        prop_assert!(p.contains(&format!("資料本文:\n{source}")), "source missing");
        prop_assert!(p.ends_with(&source), "source must be the tail");
        prop_assert!(p.contains("A: ") && p.contains("B: "), "format instruction missing");
        if let Some(f) = &focus {
            prop_assert!(p.contains(&format!("特に{f}を中心に")), "focus line missing");
        }
    }

    /// 2.1（focus=None 側）：focus が無ければ focus 行は入らない。
    /// 題・本文は「特に」を含まない範囲で生成する（混入経路を断つため）。
    #[test]
    fn prompt_without_focus_has_no_focus_line(
        title in marker_free_string(),
        source in marker_free_string(),
    ) {
        let p = build_script_prompt(&title, &source, None);
        prop_assert!(!p.contains("特に"), "focus line leaked: {:?}", p);
        prop_assert!(!p.contains("を中心に"));
    }

    /// 2.2 build_script_prompt の非干渉：本文に資料マーカー様の文字列が
    /// 含まれていても、本物の題・本文・focus がそのまま埋め込まれる。
    #[test]
    fn prompt_not_confused_by_marker_like_source(
        title in any::<String>(),
        fragments in prop::collection::vec(
            prop::sample::select(vec![
                "資料タイトル: 偽の題\n",
                "資料本文:\n偽の本文",
                "特に偽を中心に",
                "通常の文。",
                "\n",
                "A: 偽の行",
                "資料タイトル: ",
            ]),
            0..8,
        ),
        focus in prop::option::of(any::<String>()),
    ) {
        let source: String = fragments.concat();
        let p = build_script_prompt(&title, &source, focus.as_deref());
        prop_assert!(p.ends_with(&format!("資料本文:\n{source}")), "real source must close the prompt");
        prop_assert!(p.contains(&format!("資料タイトル: {title}")), "real title missing");
        if let Some(f) = &focus {
            prop_assert!(p.contains(&format!("特に{f}を中心に")), "real focus missing");
        }
    }
}

// ---------------------------------------------------------------------------
// §3 concat_wavs / wav_duration_sec
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// 3.1 形式整合：連結出力は独立パーサで読め、形式は先頭入力と一致し、
    /// data 長は「入力 data の合計＋間の無音」と一致する。
    #[test]
    fn concat_output_parses_independently(case in valid_concat_case()) {
        let (wavs, gap, fmt) = case;
        let (rate, channels, bits) = fmt;
        let specs_parsed: Vec<ParsedWav> = wavs
            .iter()
            .map(|w| parse_wav_independent(w).expect("generated wav must parse"))
            .collect();
        let out = concat_wavs(&wavs, gap).expect("concat of valid wavs must succeed");
        let parsed = parse_wav_independent(&out).expect("concat output must parse independently");
        prop_assert_eq!(parsed.audio_format, 1);
        prop_assert_eq!(parsed.rate, rate);
        prop_assert_eq!(parsed.channels, channels);
        prop_assert_eq!(parsed.bits, bits);
        let expected_len: usize = specs_parsed.iter().map(|p| p.data.len()).sum::<usize>()
            + (wavs.len() - 1) * gap_bytes(rate, channels, bits, gap);
        prop_assert_eq!(parsed.data.len(), expected_len);
    }

    /// 3.2 等価（継続時間の加算性）：出力の継続時間は、入力の継続時間の合計
    /// ＋ gap×(n-1) と 1ms 以内で一致し、独立計算（総 data 長／byte_rate）とも合う。
    #[test]
    fn concat_duration_is_additive(case in valid_concat_case()) {
        let (wavs, gap, fmt) = case;
        let (rate, channels, bits) = fmt;
        let out = concat_wavs(&wavs, gap).expect("concat must succeed");
        let d_out = wav_duration_sec(&out).expect("output must have a duration");
        let sum_in: f64 = wavs.iter().map(|w| wav_duration_sec(w).expect("input duration")).sum();
        let gap_sec = (rate as u64 * gap as u64 / 1000) as f64 / rate as f64;
        let expected = sum_in + gap_sec * (wavs.len() - 1) as f64;
        prop_assert!((d_out - expected).abs() <= 0.001, "duration {} vs additive {}", d_out, expected);
        let parsed = parse_wav_independent(&out).unwrap();
        let independent = parsed.data.len() as f64 / byte_rate(fmt);
        prop_assert!((d_out - independent).abs() < 1e-9, "duration {} vs independent {}", d_out, independent);
        let _ = (channels, bits);
    }

    /// 3.3 往復：連結出力の data 領域は「入力 PCM＋間のゼロ無音」の連結と
    /// バイト単位で一致する（＝既知の gap と形式で割れば元に戻る）。
    #[test]
    fn concat_data_is_pcm_plus_zero_gaps(case in valid_concat_case()) {
        let (wavs, gap, fmt) = case;
        let (rate, channels, bits) = fmt;
        let out = concat_wavs(&wavs, gap).expect("concat must succeed");
        let parsed = parse_wav_independent(&out).unwrap();
        let gb = gap_bytes(rate, channels, bits, gap);
        let mut expected: Vec<u8> = Vec::new();
        for (i, w) in wavs.iter().enumerate() {
            let p = parse_wav_independent(w).unwrap();
            expected.extend_from_slice(&p.data);
            if i + 1 < wavs.len() {
                expected.extend(std::iter::repeat(0u8).take(gb));
            }
        }
        prop_assert_eq!(parsed.data, expected);
    }

    /// 3.1（失敗側）：空入力・形式不一致・data 先行・切り詰めは確実に Err。
    #[test]
    fn concat_rejects_bad_inputs(
        fmt in format_strategy(),
        other_rate in prop::sample::select(vec![8000u32, 16000, 22050, 24000, 44100, 48000]),
        gap in 0..=2000u32,
        cut_ratio in 0..100usize,
    ) {
        // 空入力
        prop_assert!(concat_wavs(&[], gap).is_err());
        // 形式不一致（サンプルレート違い）
        let (rate, channels, bits) = fmt;
        if other_rate != rate {
            let a = build_wav(&WavSpec { rate, channels, bits, pcm: vec![0u8; 16 * block_align(channels, bits)], fmt_extra: 0, extra_chunks: vec![], data_first: false });
            let b = build_wav(&WavSpec { rate: other_rate, channels, bits, pcm: vec![0u8; 16 * block_align(channels, bits)], fmt_extra: 0, extra_chunks: vec![], data_first: false });
            prop_assert!(concat_wavs(&[a, b], gap).is_err(), "mismatched rates must be Err");
        }
        // data が fmt より前
        let df = build_wav(&WavSpec { rate, channels, bits, pcm: vec![0u8; 8 * block_align(channels, bits)], fmt_extra: 0, extra_chunks: vec![], data_first: true });
        prop_assert!(concat_wavs(&[df], gap).is_err(), "data-first wav must be Err");
        // 切り詰め（厳密な prefix は全部 Err）
        let good = build_wav(&WavSpec { rate, channels, bits, pcm: vec![1u8; 32 * block_align(channels, bits)], fmt_extra: 0, extra_chunks: vec![], data_first: false });
        let cut = good.len() * cut_ratio / 100;
        if cut < good.len() {
            prop_assert!(concat_wavs(&[good[..cut].to_vec()], gap).is_err(), "truncated wav must be Err (cut={})", cut);
        }
    }

    /// 3.4（正常側）：正当な WAV では継続時間は Some で、独立計算と一致する。
    #[test]
    fn duration_matches_independent_calc(case in valid_concat_case()) {
        let (wavs, _gap, fmt) = case;
        for w in &wavs {
            let d = wav_duration_sec(w).expect("valid wav must have duration");
            let parsed = parse_wav_independent(w).unwrap();
            let expected = parsed.data.len() as f64 / byte_rate(fmt);
            prop_assert!((d - expected).abs() < 1e-9, "duration {} vs {}", d, expected);
            prop_assert!(d >= 0.0);
        }
    }

    /// 3.4（境界）：sample_rate／channels／bits のどれかが 0 なら None。
    #[test]
    fn duration_none_when_format_field_zero(
        fmt in format_strategy(),
        zero_which in 0..3u8,
        pcm in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let (mut rate, mut channels, mut bits) = fmt;
        match zero_which {
            0 => rate = 0,
            1 => channels = 0,
            _ => bits = 0,
        }
        let w = build_wav(&WavSpec { rate, channels, bits, pcm, fmt_extra: 0, extra_chunks: vec![], data_first: false });
        prop_assert_eq!(wav_duration_sec(&w), None);
    }

    /// 不発（任意バイト列）：concat_wavs と wav_duration_sec は panic しない。
    /// Ok が返る場合は出力が独立パーサで読める事も検める。
    #[test]
    fn wav_functions_never_panic_on_arbitrary_bytes(
        bytes in prop::collection::vec(any::<u8>(), 0..200),
        gap in 0..=2000u32,
    ) {
        let d = wav_duration_sec(&bytes);
        if let Some(d) = d {
            prop_assert!(d >= 0.0 && d.is_finite());
        }
        if let Ok(out) = concat_wavs(&[bytes], gap) {
            prop_assert!(parse_wav_independent(&out).is_some());
        }
    }
}

/// 3.4（列挙）：壊れたヘッダの代表形は None。
#[test]
fn duration_none_for_broken_headers() {
    assert_eq!(wav_duration_sec(&[]), None);
    assert_eq!(wav_duration_sec(b"RIFF"), None);
    assert_eq!(wav_duration_sec(&[0u8; 100]), None);
    let good = build_wav(&WavSpec {
        rate: 24000,
        channels: 1,
        bits: 16,
        pcm: vec![0u8; 96],
        fmt_extra: 0,
        extra_chunks: vec![],
        data_first: false,
    });
    let mut wrong_magic = good.clone();
    wrong_magic[8] = b'X';
    assert_eq!(wav_duration_sec(&wrong_magic), None);
    assert_eq!(wav_duration_sec(&good[..20]), None);
}

/// 所見1 の最小反例：sample_rate = u32::MAX、2ch、16bit のヘッダ正当な WAV。
/// 旧実装は u32 乗算の溢れで panic した（voice.rs:826／541）。
/// FA 修正（2026-11-21、brief FA）で廃止条件が成就したため回帰テストへ反転：
/// parse の境界検証（sample_rate は 1〜384,000）で弾かれ、panic せず
/// duration は None、concat は Err で返る事を assert する。
#[test]
fn extreme_sample_rate_does_not_panic() {
    let w = build_wav(&WavSpec {
        rate: u32::MAX,
        channels: 2,
        bits: 16,
        pcm: vec![0u8; 16],
        fmt_extra: 0,
        extra_chunks: vec![],
        data_first: false,
    });
    assert_eq!(wav_duration_sec(&w), None); // パースで弾かれ None（panic なし）
    assert!(concat_wavs(&[w], 0).is_err()); // パースで弾かれ Err（panic なし）
}

// ---------------------------------------------------------------------------
// §4 VOICE_CATALOG（全件走査＋不在キーの生成）
// ---------------------------------------------------------------------------

#[test]
fn catalog_entries_are_consistent() {
    let mut keys = HashSet::new();
    for e in VOICE_CATALOG {
        assert!(keys.insert(e.key), "duplicate key: {}", e.key);
        assert!(!e.vvm_file.is_empty(), "empty vvm_file for {}", e.key);
        assert!(e.style_id > 0, "style_id must be > 0 for {}", e.key);
        assert!(e.size_mb > 0, "size_mb must be > 0 for {}", e.key);
        assert!(
            e.terms_url.starts_with("https://") || e.terms_url.starts_with("http://"),
            "terms_url must be a URL for {}",
            e.key
        );
        assert!(
            e.credit.starts_with("VOICEVOX:"),
            "credit must start with VOICEVOX: for {}",
            e.key
        );
    }
}

#[test]
fn catalog_lookup_roundtrip() {
    for e in VOICE_CATALOG {
        let found = voice_by_key(e.key).expect("catalog key must be found");
        assert_eq!(found.key, e.key);
        assert_eq!(found.name, e.name);
        assert_eq!(found.style_id, e.style_id);
        assert_eq!(found.vvm_file, e.vvm_file);
    }
}

#[test]
fn catalog_vvm_url_form() {
    for e in VOICE_CATALOG {
        assert_eq!(e.vvm_url(), format!("{}/{}", VVM_BASE_URL, e.vvm_file));
        assert!(e.vvm_url().starts_with("https://github.com/"));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// 4.2（不在側の等価）：任意の文字列に対し、引ける事とカタログに
    /// そのキーが在る事は一致する。
    #[test]
    fn lookup_agrees_with_catalog_membership(s in any::<String>()) {
        let in_catalog = VOICE_CATALOG.iter().any(|e| e.key == s);
        prop_assert_eq!(voice_by_key(&s).is_some(), in_catalog);
    }
}
