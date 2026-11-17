# MemoriaNotebook

Gemini Notebook（旧 NotebookLM）の芯——資料の束＋検索＋出典縛りの対話——を
Memoria の部品で組み、端末で完結させる Android アプリの試作リポジトリ。

## 決まっている形

- **B案**：核（記録層／検索／llama.cpp）を共有する別アプリとして試作し、
  使い物になれば Memoria 本体の一画面として統合する。
- **範囲**：ノート／ソース／出典付き対話＋音声概要のみ。他の Studio 生成物は作らない。
- **モデル座**：対話はローカルが標準。API は設定で刺す opt-in の座で、
  接続先・キー・モデル名は利用者が自由に設定する（BYOK 無制限）。
  音声概要の台本生成だけは API 前提（ローカルには作らせない）。
  音声合成は VOICEVOX（核の外、アプリ側）。
- **二大原則**：Rust が基本／複雑よりシンプル。

## 現在の段

- `crates/notebook-core`：核の土台。`CoreState` の界面と記録層
  （`notebooks.jsonl`／`sources.jsonl`／`turns.jsonl`）＋チャンク側車
  （`chunks.jsonl`、段落単位・約400字、版管理付き）＋埋め込み側車
  （`emb_chunks.bin`）とノート範囲に絞ったコサイン検索。埋め込み器は
  トレイトで差し替え可能（実モデルは後段でホスト側が差す）。
  質問→検索→根拠→引用付き回答の最小E2Eが `tests/e2e.rs` で通しで
  回る。推論の座は `Generator` トレイトと `CoreState::ask` が立ち、
  実モデル（ローカル llama／API）はホスト側が差す形になった。
