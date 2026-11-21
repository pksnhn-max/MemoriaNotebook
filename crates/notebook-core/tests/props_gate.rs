//! 性質テスト：出典・引用ゲート（annex `props-gate-voice.md` §1・§2 の全候補）。
//!
//! - §1：`format_grounding` の等価・往復、`parse_citations` の等価・不発・形式整合。
//!   期待値は仕様文（annex §1.1）から独立に書いたオラクルで計算する（実装と共有しない）。
//! - §2：引用ゲート（`ask_generate` / `ask_generate_with_sink` / `ask_commit`）。
//!   仕様（確定裁定）：根拠があるのに有効出典ゼロ → 再構築して1回だけ再試行 →
//!   なおゼロなら固定拒否「資料にありません。」。ストリーミングは Token/Reset の
//!   列だけを対象にする（断片の粒度・順序の細部は対象外）。
//!
//! ケース数の床は全性質 1,000（brief の裁定。annex の 200／500 行も引き上げ済み）。

use notebook_core::{
    ask_generate, ask_generate_with_sink, format_grounding, parse_citations, AskPrepared, ChatTurn,
    ChunkRec, Citation, CoreError, CoreResult, CoreState, EngineCfg, GeneratedAnswer, Generator,
    Role, StreamEvent,
};
use proptest::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// 共通の生成器
// ---------------------------------------------------------------------------

fn chunk_rec(id: u64, source_id: u64, text: String) -> ChunkRec {
    ChunkRec {
        id,
        source_id,
        nb: 0,
        ord: 0,
        rev: 1,
        text,
    }
}

/// id が重複しないヒット列。text は任意文字列（改行・マーカー様文字列を含む）。
fn hits_strategy() -> impl Strategy<Value = Vec<(ChunkRec, f32)>> {
    prop::collection::hash_set(any::<u64>(), 0..=6).prop_flat_map(|id_set| {
        let ids: Vec<u64> = id_set.into_iter().collect();
        let n = ids.len();
        (
            prop::collection::vec(any::<u64>(), n..=n),
            prop::collection::vec(any::<String>(), n..=n),
            prop::collection::vec(prop::num::f32::ANY, n..=n),
        )
            .prop_map(move |(sources, texts, scores)| {
                ids.iter()
                    .cloned()
                    .zip(sources)
                    .zip(texts)
                    .zip(scores)
                    .map(|(((id, sid), text), score)| (chunk_rec(id, sid, text), score))
                    .collect()
            })
    })
}

/// id が下限以上で重複しないヒット列（未知マーカーと衝突させない用）。
/// 件数は `min_count..=6`。
fn hits_with_ids_from(min: u64, min_count: usize) -> impl Strategy<Value = Vec<(ChunkRec, f32)>> {
    prop::collection::hash_set(min..u64::MAX, min_count..=6).prop_flat_map(|id_set| {
        let ids: Vec<u64> = id_set.into_iter().collect();
        let n = ids.len();
        (
            prop::collection::vec(any::<u64>(), n..=n),
            prop::collection::vec("[^\n]{0,40}", n..=n),
        )
            .prop_map(move |(sources, texts)| {
                ids.iter()
                    .cloned()
                    .zip(sources)
                    .zip(texts)
                    .map(|((id, sid), text)| (chunk_rec(id, sid, text), 1.0f32))
                    .collect()
            })
    })
}

// ---------------------------------------------------------------------------
// §1 のオラクル（仕様文から独立に書いたもの。実装とは共有しない）
// ---------------------------------------------------------------------------

/// `format_grounding` の仕様：空なら空文字列、そうでなければ
/// `[#<id>] <text>` を空行1つ（"\n\n"）で連結したもの。
fn oracle_grounding(hits: &[(ChunkRec, f32)]) -> String {
    let mut s = String::new();
    for (i, (c, _)) in hits.iter().enumerate() {
        if i > 0 {
            s.push_str("\n\n");
        }
        s.push_str("[#");
        s.push_str(&c.id.to_string());
        s.push_str("] ");
        s.push_str(&c.text);
    }
    s
}

/// `format_grounding` の出力を読み戻す独立パーサ（往復 1.2 用）。
/// ブロックの区切りは「"\n\n" の直後に `[#<数字>] ` が続く位置」だけを数える。
/// それ以外の "\n\n" は本文の一部として扱う。
fn parse_grounding_independent(s: &str) -> Option<Vec<(u64, String)>> {
    if s.is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    let mut rest = s;
    loop {
        let after = rest.strip_prefix("[#")?;
        let close = after.find("] ")?;
        let id_str = &after[..close];
        if id_str.is_empty() || !id_str.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let id: u64 = id_str.parse().ok()?;
        let body = &after[close + 2..];
        match find_block_separator(body) {
            Some(idx) => {
                out.push((id, body[..idx].to_string()));
                rest = &body[idx + 2..];
            }
            None => {
                out.push((id, body.to_string()));
                return Some(out);
            }
        }
    }
}

/// `body` の中で、次のブロックの開始（"\n\n[#<数字>] "）の位置を探す。
fn find_block_separator(body: &str) -> Option<usize> {
    let bytes = body.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = body[from..].find("\n\n[#") {
        let cand = from + rel;
        let mut j = cand + 4;
        let mut digits = 0usize;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            digits += 1;
            j += 1;
        }
        if digits > 0 && j + 1 < bytes.len() && bytes[j] == b']' && bytes[j + 1] == b' ' {
            return Some(cand);
        }
        from = cand + 1;
    }
    None
}

/// `parse_citations` の仕様オラクル（annex §1.1 の規則文から書いた）。
/// `[#` を探し、数字＋`]` が続けば印として消費する。数字は saturating で畳む
/// （仕様文の「数字」の自然な読み。u64 を超える印は MAX に張り付く）。
/// 実在 id のみ採用、出現順、重複は初回のみ。
fn oracle_citations(text: &str, hits: &[(ChunkRec, f32)]) -> Vec<Citation> {
    let mut source_of: HashMap<u64, u64> = HashMap::new();
    for (c, _) in hits {
        source_of.insert(c.id, c.source_id);
    }
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut seen: HashSet<u64> = HashSet::new();
    let mut pos = 0usize;
    while let Some(rel) = text[pos..].find("[#") {
        let start = pos + rel;
        let mut j = start + 2;
        let mut num: u64 = 0;
        let mut digits = 0usize;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            num = num
                .saturating_mul(10)
                .saturating_add((bytes[j] - b'0') as u64);
            digits += 1;
            j += 1;
        }
        if digits > 0 && j < bytes.len() && bytes[j] == b']' {
            if let Some(&sid) = source_of.get(&num) {
                if seen.insert(num) {
                    out.push(Citation {
                        source_id: sid,
                        chunk_id: Some(num),
                    });
                }
            }
            pos = j + 1;
        } else {
            // 印として成立しなかった "[#" の直後から探し直す。
            // 次の候補は早くても start+2（start+1 は '#' で印は始まらない）。
            pos = start + 2;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// テキスト部品の生成（§1・§2 で共用）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Piece {
    Filler(String),
    Marker(String),
}

fn filler_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec![
            'a', 'b', 'Z', ' ', '[', ']', '#', '1', '2', '\n', '、', '。', 'あ', '\t',
        ]),
        0..10,
    )
    .prop_map(|cs| cs.into_iter().collect())
}

fn non_numeric_marker_strategy() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "[#abc]", "[#]", "[# 12]", "[#12 ]", "[#１２]", "[#1.5]", "[#-3]", "[##]", "[#", "[#x9]",
        " [#]",
    ])
    .prop_map(|s| s.to_string())
}

/// `known`（実在 id 列）に対する部品列。valid／unknown／非数値／filler を混ぜる。
fn pieces_strategy(known: Vec<u64>) -> impl Strategy<Value = Vec<Piece>> {
    let known_set: HashSet<u64> = known.iter().cloned().collect();
    let base = prop_oneof![
        3 => filler_strategy().prop_map(Piece::Filler),
        2 => non_numeric_marker_strategy().prop_map(Piece::Marker),
        3 => any::<u64>().prop_map(move |n| {
            // 実在 id と衝突したら未知側の意味を保つためずらす
            let mut m = n;
            if known_set.contains(&m) {
                m = m.wrapping_add(1);
            }
            if known_set.contains(&m) {
                m = m.wrapping_add(1);
            }
            Piece::Marker(format!("[#{m}]"))
        }),
    ];
    let piece = if known.is_empty() {
        base.boxed()
    } else {
        prop_oneof![
            4 => base,
            3 => prop::sample::select(known).prop_map(|id| Piece::Marker(format!("[#{id}]"))),
        ]
        .boxed()
    };
    prop::collection::vec(piece, 0..14)
}

fn join_pieces(pieces: &[Piece]) -> String {
    let seps = ["", " ", "\n", "、", "。", "x"];
    let mut s = String::new();
    for (i, p) in pieces.iter().enumerate() {
        if i > 0 {
            s.push_str(seps[i % seps.len()]);
        }
        match p {
            Piece::Filler(t) | Piece::Marker(t) => s.push_str(t),
        }
    }
    s
}

/// (hits, それに対するテキスト) の対。
fn hits_and_text_strategy() -> impl Strategy<Value = (Vec<(ChunkRec, f32)>, String)> {
    hits_strategy().prop_flat_map(|hits| {
        let known: Vec<u64> = hits.iter().map(|(c, _)| c.id).collect();
        let h = hits.clone();
        pieces_strategy(known).prop_map(move |pieces| (h.clone(), join_pieces(&pieces)))
    })
}

/// 有効出典ゼロになる事が保証されるテキスト（数値マーカーは全て未知側）。
/// hits の id は 1_000_000 以上に寄せ、テキスト側の数値マーカーは
/// 1_000_000 未満しか出さない。filler の数字も '1' '2' だけで 7 桁には届かない。
fn hits_and_uncited_strategy() -> impl Strategy<Value = (Vec<(ChunkRec, f32)>, String)> {
    hits_with_ids_from(1_000_000, 0).prop_flat_map(|hits| {
        let h = hits.clone();
        let piece = prop_oneof![
            3 => filler_strategy().prop_map(Piece::Filler),
            2 => non_numeric_marker_strategy().prop_map(Piece::Marker),
            3 => (0..1_000_000u64).prop_map(|n| Piece::Marker(format!("[#{n}]"))),
        ];
        prop::collection::vec(piece, 0..14)
            .prop_map(move |pieces| (h.clone(), join_pieces(&pieces)))
    })
}

// ---------------------------------------------------------------------------
// §2 のフェイク生成器と観測
// ---------------------------------------------------------------------------

/// 応答をキューで完全制御する生成器。`generate_streaming` は応答を
/// 最大3片に割って流す（粒度その物は性質の対象外）。
#[derive(Debug)]
struct QueueGenerator {
    responses: VecDeque<CoreResult<String>>,
    prompts: Vec<String>,
}

impl QueueGenerator {
    fn new(responses: Vec<CoreResult<String>>) -> Self {
        Self {
            responses: responses.into(),
            prompts: Vec::new(),
        }
    }
    fn next_response(&mut self, prompt: &str) -> CoreResult<String> {
        self.prompts.push(prompt.to_string());
        self.responses.pop_front().unwrap_or_else(|| {
            Err(CoreError::GeneratorError(
                "unscripted generator call".to_string(),
            ))
        })
    }
}

impl Generator for QueueGenerator {
    fn generate(&mut self, prompt: &str) -> CoreResult<String> {
        self.next_response(prompt)
    }
    fn generate_streaming(
        &mut self,
        prompt: &str,
        on_piece: &mut dyn FnMut(&str),
    ) -> CoreResult<String> {
        let text = self.next_response(prompt)?;
        for piece in split_into_pieces(&text) {
            on_piece(piece);
        }
        Ok(text)
    }
}

/// `generate` だけを実装し、`generate_streaming` はトレイト既定実装に任せる生成器（2.6 用）。
#[derive(Debug)]
struct GenerateOnlyGenerator {
    responses: VecDeque<CoreResult<String>>,
    prompts: Vec<String>,
}

impl GenerateOnlyGenerator {
    fn new(responses: Vec<CoreResult<String>>) -> Self {
        Self {
            responses: responses.into(),
            prompts: Vec::new(),
        }
    }
}

impl Generator for GenerateOnlyGenerator {
    fn generate(&mut self, prompt: &str) -> CoreResult<String> {
        self.prompts.push(prompt.to_string());
        self.responses.pop_front().unwrap_or_else(|| {
            Err(CoreError::GeneratorError(
                "unscripted generator call".to_string(),
            ))
        })
    }
}

/// テキストを文字境界で最大3片に割る。
fn split_into_pieces(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let bounds: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    let n = bounds.len() - 1; // 文字数
    if n <= 1 {
        return vec![text];
    }
    let c1 = bounds[n / 3];
    let c2 = bounds[2 * n / 3];
    let mut out = Vec::new();
    if c1 > 0 {
        out.push(&text[..c1]);
    }
    if c2 > c1 {
        out.push(&text[c1..c2]);
    }
    out.push(&text[c2..]);
    out.retain(|p| !p.is_empty());
    out
}

/// sink へ届いたイベントの観測列。対象は Token/Reset の列だけ。
#[derive(Debug, PartialEq)]
enum Ev {
    T(String),
    R,
}

fn run_gate(
    prepared: &AskPrepared,
    gen: &mut QueueGenerator,
) -> (CoreResult<GeneratedAnswer>, Vec<Ev>) {
    let mut events = Vec::new();
    let r = ask_generate_with_sink(prepared, gen, &mut |e| match e {
        StreamEvent::Token(s) => events.push(Ev::T(s)),
        StreamEvent::Reset => events.push(Ev::R),
    });
    (r, events)
}

/// イベント列の形（"TTRTR" のような文字列）。
fn kinds(events: &[Ev]) -> String {
    events
        .iter()
        .map(|e| match e {
            Ev::T(_) => 'T',
            Ev::R => 'R',
        })
        .collect()
}

/// Reset で区切った試行ごとのトークン連結。
fn attempt_texts(events: &[Ev]) -> Vec<String> {
    let mut out = vec![String::new()];
    for e in events {
        match e {
            Ev::T(s) => out.last_mut().unwrap().push_str(s),
            Ev::R => out.push(String::new()),
        }
    }
    out
}

fn prepared(hits: Vec<(ChunkRec, f32)>, question: &str) -> AskPrepared {
    let grounding = format_grounding(&hits);
    AskPrepared {
        nb: 7,
        question: question.to_string(),
        hits,
        grounding,
        prompt: "FIRST-PROMPT".to_string(),
        retry_prompt: "RETRY-PROMPT".to_string(),
    }
}

/// hits に対する「引用あり」テキスト：実在 id の印を最低1つ含む。
fn cited_text_strategy(hits: &[(ChunkRec, f32)]) -> impl Strategy<Value = String> {
    let ids: Vec<u64> = hits.iter().map(|(c, _)| c.id).collect();
    let n = ids.len();
    (
        prop::collection::vec(0..n, 1..=5),
        prop::collection::vec(filler_strategy(), 1..=6),
    )
        .prop_map(move |(picks, fillers)| {
            let mut s = String::new();
            for (i, pick) in picks.iter().enumerate() {
                if i < fillers.len() {
                    s.push_str(&fillers[i]);
                }
                s.push_str(&format!("[#{}]", ids[*pick]));
                s.push(' ');
            }
            s
        })
}

/// (gate用 hits, 引用なしテキスト, 引用ありテキスト) の組。
fn gate_case_strategy(
) -> impl Strategy<Value = (Vec<(ChunkRec, f32)>, String, String)> {
    hits_with_ids_from(1_000_000, 1).prop_flat_map(|hits| {
        let h = hits.clone();
        // 引用なし側：数値マーカーは 1_000_000 未満のみ（hits と衝突しない）
        let uncited_piece = prop_oneof![
            3 => filler_strategy().prop_map(Piece::Filler),
            2 => non_numeric_marker_strategy().prop_map(Piece::Marker),
            3 => (0..1_000_000u64).prop_map(|n| Piece::Marker(format!("[#{n}]"))),
        ];
        let uncited = prop::collection::vec(uncited_piece, 0..10).prop_map(|ps| join_pieces(&ps));
        let cited = cited_text_strategy(&h);
        (uncited, cited).prop_map(move |(u, c)| (h.clone(), u, c))
    })
}

// ---------------------------------------------------------------------------
// §1 性質
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// 1.1 format_grounding の等価：出力は仕様通りの組み立てと一致する。
    #[test]
    fn grounding_matches_spec_construction(hits in hits_strategy()) {
        prop_assert_eq!(format_grounding(&hits), oracle_grounding(&hits));
    }

    /// 1.2 format_grounding の往復：独立パーサで読み戻すと元の (id, text) 列に戻り、
    /// 再適用すると同じ文字列になる。
    /// 領域の留保：本文に「"\n\n[#<数字>] "」その物が含まれると書式自体が曖昧になる
    /// （区切りと本文が区別できない）ため、生成時にその衝突だけを取り除く。
    /// これは書式の性質であって実装の不成立ではない（記録帳に明記）。
    #[test]
    fn grounding_roundtrip_via_independent_parser(hits in hits_strategy()) {
        let hits: Vec<(ChunkRec, f32)> = hits
            .into_iter()
            .map(|(mut c, s)| {
                c.text = c.text.replace("\n\n[#", "\n\n[＃");
                (c, s)
            })
            .collect();
        let formatted = format_grounding(&hits);
        let parsed = parse_grounding_independent(&formatted);
        prop_assert!(parsed.is_some(), "independent parser rejected: {:?}", formatted);
        let parsed = parsed.unwrap();
        let expected: Vec<(u64, String)> =
            hits.iter().map(|(c, _)| (c.id, c.text.clone())).collect();
        prop_assert_eq!(&parsed, &expected);
        // 再適用：読み戻した列を組み立て直すと元の文字列と一致する。
        let rebuilt: Vec<(ChunkRec, f32)> = parsed
            .iter()
            .map(|(id, text)| (chunk_rec(*id, 0, text.clone()), 0.0))
            .collect();
        prop_assert_eq!(format_grounding(&rebuilt), formatted);
    }

    /// 1.3 parse_citations の等価：仕様オラクルと完全一致する。
    /// 1.5 形式整合（出現順・重複除去）はオラクル一致に加え、
    /// 結果の chunk_id が重複せず、全て hits の実在 id で source_id が
    /// 逆引きと合う事を直接にも検める。
    #[test]
    fn citations_match_spec_oracle(case in hits_and_text_strategy()) {
        let (hits, text) = case;
        let got = parse_citations(&text, &hits);
        prop_assert_eq!(&got, &oracle_citations(&text, &hits));
        let mut seen = HashSet::new();
        for cit in &got {
            let cid = cit.chunk_id.expect("citation must carry chunk_id");
            prop_assert!(seen.insert(cid), "duplicate chunk_id in result");
            let hit = hits.iter().find(|(c, _)| c.id == cid).expect("unknown id adopted");
            prop_assert_eq!(cit.source_id, hit.0.source_id);
        }
    }

    /// 1.4 parse_citations の不発：有効印ゼロのテキストでは常に空ベクタ。
    /// hits は空・非空の両方を生成する。
    #[test]
    fn citations_empty_when_no_valid_markers(case in hits_and_uncited_strategy()) {
        let (hits, text) = case;
        let got = parse_citations(&text, &hits);
        prop_assert!(got.is_empty(), "expected no citations, got {:?}", got);
    }
}

// ---------------------------------------------------------------------------
// §2 性質
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// 2.1 ゲート：初回で有効出典あり → そのまま採用・リトライなし・Resetなし。
    #[test]
    fn gate_accepts_first_answer_with_citations(case in gate_case_strategy()) {
        let (hits, _uncited, cited) = case;
        let p = prepared(hits.clone(), "質問");
        let mut gen = QueueGenerator::new(vec![Ok(cited.clone())]);
        let (r, events) = run_gate(&p, &mut gen);
        let ans = r.expect("gate must succeed");
        prop_assert_eq!(&ans.text, &cited);
        prop_assert_eq!(&ans.citations, &oracle_citations(&cited, &hits));
        prop_assert!(!ans.citations.is_empty());
        prop_assert_eq!(gen.prompts, vec!["FIRST-PROMPT".to_string()]);
        prop_assert!(!kinds(&events).contains('R'), "unexpected Reset: {}", kinds(&events));
        prop_assert_eq!(attempt_texts(&events), vec![cited.clone()]);
    }

    /// 2.2 ゲート：初回出典ゼロ・ヒットあり → Reset1回・リトライ1回 →
    /// 2回目で出典あり → 採用。
    #[test]
    fn gate_retries_once_and_accepts(case in gate_case_strategy()) {
        let (hits, uncited, cited) = case;
        let p = prepared(hits.clone(), "質問");
        let mut gen = QueueGenerator::new(vec![Ok(uncited.clone()), Ok(cited.clone())]);
        let (r, events) = run_gate(&p, &mut gen);
        let ans = r.expect("gate must succeed");
        prop_assert_eq!(&ans.text, &cited);
        prop_assert_eq!(&ans.citations, &oracle_citations(&cited, &hits));
        prop_assert_eq!(
            gen.prompts,
            vec!["FIRST-PROMPT".to_string(), "RETRY-PROMPT".to_string()]
        );
        let ks = kinds(&events);
        prop_assert_eq!(ks.matches('R').count(), 1, "kinds: {}", ks);
        prop_assert_eq!(attempt_texts(&events), vec![uncited.clone(), cited.clone()]);
    }

    /// 2.3 ゲート：2回とも出典ゼロ → 固定拒否「資料にありません。」、
    /// citations 空、Reset は2回（リトライ前と拒否時）。
    #[test]
    fn gate_refuses_after_one_retry(case in gate_case_strategy()) {
        let (hits, uncited, _cited) = case;
        let uncited2 = uncited.clone();
        let p = prepared(hits.clone(), "質問");
        let mut gen = QueueGenerator::new(vec![Ok(uncited.clone()), Ok(uncited2)]);
        let (r, events) = run_gate(&p, &mut gen);
        let ans = r.expect("gate must succeed");
        prop_assert_eq!(ans.text, "資料にありません。".to_string());
        prop_assert!(ans.citations.is_empty());
        prop_assert_eq!(
            gen.prompts,
            vec!["FIRST-PROMPT".to_string(), "RETRY-PROMPT".to_string()]
        );
        let ks = kinds(&events);
        prop_assert_eq!(ks.matches('R').count(), 2, "kinds: {}", ks);
        prop_assert!(ks.ends_with('R'), "refusal must end with Reset: {}", ks);
    }

    /// 2.4 ゲート：ヒットなし → 初回で出典ゼロでもリトライせず素通し。
    #[test]
    fn gate_passes_through_when_no_hits(text in any::<String>()) {
        let p = prepared(Vec::new(), "質問");
        let mut gen = QueueGenerator::new(vec![Ok(text.clone())]);
        let (r, events) = run_gate(&p, &mut gen);
        let ans = r.expect("gate must succeed");
        prop_assert_eq!(&ans.text, &text);
        prop_assert!(ans.citations.is_empty());
        prop_assert_eq!(gen.prompts, vec!["FIRST-PROMPT".to_string()]);
        prop_assert!(!kinds(&events).contains('R'));
    }

    /// 2.5 ゲート：生成器エラー → CoreError::GeneratorError が伝播し、
    /// sink へは何も流れない。
    #[test]
    fn gate_propagates_generator_error(case in gate_case_strategy()) {
        let (hits, _u, _c) = case;
        let p = prepared(hits, "質問");
        // ask_generate（sink なし）
        let mut gen = QueueGenerator::new(vec![Err(CoreError::GeneratorError("boom".to_string()))]);
        let r = ask_generate(&p, &mut gen);
        prop_assert!(matches!(r, Err(CoreError::GeneratorError(_))), "got {:?}", r);
        prop_assert_eq!(gen.prompts.len(), 1);
        // ask_generate_with_sink（sink は空のはず）
        let mut gen2 = QueueGenerator::new(vec![Err(CoreError::GeneratorError("boom".to_string()))]);
        let (r2, events) = run_gate(&p, &mut gen2);
        prop_assert!(matches!(r2, Err(CoreError::GeneratorError(_))), "got {:?}", r2);
        prop_assert!(events.is_empty(), "sink must stay silent on error: {:?}", events);
    }

    /// 2.6 ストリーミング既定実装：`generate` のみ実装の生成器でも、
    /// 明示ストリーミング生成器と最終結果・Reset 回数・プロンプト列が一致する。
    #[test]
    fn gate_default_streaming_matches_explicit(case in gate_case_strategy(), scenario in 0..4u8) {
        let (hits, uncited, cited) = case;
        let (hits, script): (Vec<(ChunkRec, f32)>, Vec<String>) = match scenario {
            0 => (hits, vec![cited]),
            1 => (hits, vec![uncited, cited]),
            2 => (hits, vec![uncited.clone(), uncited]),
            _ => (Vec::new(), vec![cited]),
        };
        let p = prepared(hits, "質問");
        let mut g1 = QueueGenerator::new(script.iter().cloned().map(Ok).collect());
        let (r1, ev1) = run_gate(&p, &mut g1);
        let mut g2 = GenerateOnlyGenerator::new(script.iter().cloned().map(Ok).collect());
        let mut resets2 = 0usize;
        let r2 = ask_generate_with_sink(&p, &mut g2, &mut |e| {
            if matches!(e, StreamEvent::Reset) {
                resets2 += 1;
            }
        });
        let a1 = r1.expect("explicit streaming must succeed");
        let a2 = r2.expect("default streaming must succeed");
        prop_assert_eq!(&a1.text, &a2.text);
        prop_assert_eq!(&a1.citations, &a2.citations);
        prop_assert_eq!(g1.prompts, g2.prompts);
        let resets1 = ev1.iter().filter(|e| matches!(e, Ev::R)).count();
        prop_assert_eq!(resets1, resets2);
    }
}

// ---------------------------------------------------------------------------
// 2.7 ask_commit の往復（ファイル I/O を伴うため通常の test として書く）
// ---------------------------------------------------------------------------

static SEQ: AtomicU64 = AtomicU64::new(0);

fn fresh_state() -> (CoreState, PathBuf) {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("nbprops-gate-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    let state = CoreState::new(&dir, EngineCfg::default(), "test-model")
        .expect("CoreState::new must succeed");
    (state, dir)
}

fn citation_strategy() -> impl Strategy<Value = Citation> {
    (any::<u64>(), prop::option::of(any::<u64>()))
        .prop_map(|(source_id, chunk_id)| Citation { source_id, chunk_id })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// 2.7 ask_commit：User→Assistant の順でターンが記録され、
    /// turns_of で読み戻すと順序・内容・citations・時刻が一致する。
    #[test]
    fn ask_commit_roundtrip(
        nb in 1..10_000u64,
        rounds in prop::collection::vec(
            (
                any::<String>(),
                any::<String>(),
                prop::collection::vec(citation_strategy(), 0..5),
                any::<u64>(),
            ),
            1..=3,
        ),
    ) {
        let (mut state, dir) = fresh_state();
        let mut expected: Vec<ChatTurn> = Vec::new();
        for (question, text, citations, at) in &rounds {
            let p = AskPrepared {
                nb,
                question: question.clone(),
                hits: Vec::new(),
                grounding: String::new(),
                prompt: "P".to_string(),
                retry_prompt: "R".to_string(),
            };
            let ans = GeneratedAnswer {
                text: text.clone(),
                citations: citations.clone(),
            };
            let returned = state
                .ask_commit(p, ans, *at)
                .expect("ask_commit must succeed");
            let user = ChatTurn {
                nb,
                role: Role::User,
                text: question.clone(),
                citations: Vec::new(),
                at: *at,
            };
            let assistant = ChatTurn {
                nb,
                role: Role::Assistant,
                text: text.clone(),
                citations: citations.clone(),
                at: *at,
            };
            prop_assert_eq!(&returned, &assistant);
            expected.push(user);
            expected.push(assistant);
        }
        let read_back = state.turns_of(nb).expect("turns_of must succeed");
        prop_assert_eq!(read_back, expected);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
