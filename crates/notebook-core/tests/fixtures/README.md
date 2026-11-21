# W3 固定コーパスフィクスチャ（台帳・sidecar・WAV）

生成：`python3 generate_fixtures.py`（決定的。再生成で同じバイト列になる。
ただし WAV の正弦波サンプルは i16 への丸めで、テストはゴールデン比較を
しないため再生差異は無害）。

設計の出所は corpus-b1（台帳・sidecar）／corpus-b2（WAV）の annex。
ただし annex は実在しない API・スキーマを前提に書かれていたため、
中身は実コードの形式に翻訳してある。翻訳の要点：

## 台帳（ledger/）

- 実形式：chunks.jsonl は ChunkRec の JSON 行
  （`id`/`source_id`/`nb`/`ord`/`rev`/`text`）。annex の
  `{"id","text","embedding_id"}` スキーマは実コードに無い。
- tombstone は同一ファイル内の `deleted:true` 行ではなく、
  `deleted.jsonl` への裸の u64（ノートID）の追記が実形式。
  32／33／71 は chunks ファイルと deleted ファイルのペアで表す。
  削除の適用（ノート単位の除外）は CoreState の仕事で、既存単体
  テストが固定している。ここでは読み手の額面だけを固定する。
- annex の `34_dup_embedding_id` は実スキーマに対応物が無い
  （embedding_id フィールド自体が無い）ため作らない。sidecar の
  重複は emb/30・31 が担う。
- カテゴリ8（スキップ集合）は `read_jsonl` に該当パラメータが無い。
  台帳側 80〜82 は通常のファイルとして読みで固定し、スキップの
  次元は sidecar 側（emb/80〜82、`load_embeddings` の現行チャンク
  選択）で当てる。
- 固定テストは `src/oracle.rs` の `fixture_tests`。live 読み手
  （`read_jsonl`）と、テスト内に書いた独立した最小読み手
  （スキーマから手でフィールドを抜く実装）の一致を見る。
- 83（未定義バケツ）：不足フィールド行はスキップ、多余フィールド
  行は通常レコードとして読まれるのが実セマンティクスだが、annex の
  バケツ分けに従い、テストの assert は「panic しない事」だけ。

## sidecar（emb/）

- 実形式：レコード＝ chunk_id u64 LE、dim u32 LE、f32[dim] LE。
- 実読み手 `load_embeddings` の額面：逐次歩行し、**最初の不完全
  レコードで停止**する（annex の「スキップして再同期」は実在しない。
  emb/21・23・51 の中間破損は後続を巻き込む形で固定される）。
  現行チャンクIDのみ保持し、同IDは後勝ち。
- emb/40（先頭破損）は「以降2件正常」ではなく「0件で停止」が
  実額面。41 は {10} のみ、42 は {10,11}。
- 83（dim=0）は未定義バケツ扱い（実額面は空ベクトルとして読める）。

## WAV（wav/、wav/rejects/）

- 固定テストは `src/voice/oracle.rs` の `fixture_tests`。定義済み
  ケースは live vs oracle のバイト一致＋独立した固定オフセット
  パーサでのフレーム数確認。
- annex 期待値と実コードの食い違い（期待値としては固定しない）：
  - 空リスト → annex「44バイト空WAV」／実コードは Err("no wavs to concat")。
  - 単体＋gap → annex「末尾に無音追加」／実コードは gap を WAV の
    間にだけ挟むため単体では何も付かない。
  - gap_10sec ケースは2本の間に10秒を挟む形で当てる（総10.2秒）。
- rejects/ は未定義バケツ：assert は「live／oracle とも panic
  しない事」だけ。現行の額面（ソースより）：RIFF/WAVE 不正 →
  Err("not a valid RIFF/WAVE")、5バイトのテキスト → Err("wav too
  short")、header_only → Err("data chunk not found")、data サイズ
  過大・途中切断 → Err("chunk exceeds file size")、形式混在 →
  Err("wav {i} format mismatch")、32bit float → Err("only PCM
  format supported")、bad_fmt_size（fmt 18バイト）は実は受理される。
- gap_1hour と long_near_2gb は annex 自身が仕様メモ／スキップ
  指定のため生成しない。long_10min（28,800,044バイト）は生成する。
- reject_head/mid/tail/all はリストの組合せであり単体ファイルでは
  ないため、テスト内で good_100ms.wav と rejects 各ファイルを
  組み合わせて当てる。
