//! Host core E2E: the paths the single existing e2e test does not
//! walk end to end — EPUB extraction feeding the core, re-chunking a
//! source (old chunks and their citations die), and notebook scope
//! (another notebook's chunks and citations never leak in).

mod common;

use common::{temp_dir, BowEmbedder, QueueGenerator};
use notebook_core::{ask_generate_with_sink, CoreState, Embedder, EngineCfg, SourceKind};
use std::io::Write;

/// A minimal two-chapter EPUB, built the same way notebook-extract's
/// own unit tests build theirs. Chapter 1 is the cat topic with a ruby
/// annotation; chapter 2 is the dog topic.
fn minimal_epub() -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::write::ZipWriter::new(&mut buf);
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        w.start_file("mimetype", stored).unwrap();
        w.write_all(b"application/epub+zip").unwrap();
        w.start_file("META-INF/container.xml", deflated).unwrap();
        w.write_all(
            br#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
        )
        .unwrap();
        w.start_file("OEBPS/content.opf", deflated).unwrap();
        w.write_all(
            br#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf"><manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="c2.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#,
        )
        .unwrap();
        let ch1 = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>{}</p></body></html>",
            "猫 は <ruby>魚<rt>さかな</rt></ruby> が 好き で 昼寝 を する 。 ".repeat(30)
        );
        w.start_file("OEBPS/c1.xhtml", deflated).unwrap();
        w.write_all(ch1.as_bytes()).unwrap();
        let ch2 = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>{}</p></body></html>",
            "犬 は 公園 を 散歩 し て 走る 。 ".repeat(30)
        );
        w.start_file("OEBPS/c2.xhtml", deflated).unwrap();
        w.write_all(ch2.as_bytes()).unwrap();
        w.finish().unwrap();
    }
    buf.into_inner()
}

#[test]
fn epub_extracts_into_the_core_and_answers_with_a_citation() {
    // Extraction is real: spine order kept, ruby reading dropped.
    let text = notebook_extract::extract_epub(&minimal_epub()).unwrap();
    assert!(text.contains("猫 は 魚 が 好き"), "{text}");
    assert!(!text.contains("さかな"), "ruby leaked: {text}");
    assert!(
        text.find("猫").unwrap() < text.find("犬").unwrap(),
        "spine order lost: {text}"
    );

    // The extracted text walks the whole core path.
    let dir = temp_dir("epub");
    let mut st = CoreState::new(&dir, EngineCfg::default(), "w5-epub").unwrap();
    let nb = st.create_notebook("EPUB", 1).unwrap();
    let src = st
        .add_source(nb.id, SourceKind::Text, "book", "/book.epub", 2)
        .unwrap();
    let chunks = st.set_source_text(src.id, &text).unwrap();
    assert_eq!(chunks.len(), 2, "one chunk per chapter: {chunks:?}");
    assert!(chunks[0].text.contains("猫"));
    assert!(chunks[1].text.contains("犬"));
    let mut emb = BowEmbedder { dim: 64 };
    assert_eq!(st.index_source(src.id, &mut emb).unwrap(), 2);

    let prepared = st.ask_prepare(nb.id, "猫 魚", &mut emb, 2).unwrap();
    assert!(prepared.hits[0].0.text.contains("猫"));
    let top_id = prepared.hits[0].0.id;
    let mut gen = QueueGenerator::new(
        vec![format!("猫は魚が好きです[#{top_id}]。")],
        0,
    );
    let answer = ask_generate_with_sink(&prepared, &mut gen, &mut |_| {}).unwrap();
    assert_eq!(answer.citations.len(), 1);
    assert_eq!(answer.citations[0].chunk_id, Some(top_id));

    st.ask_commit(prepared, answer, 10).unwrap();
    drop(st);
    let st2 = CoreState::new(&dir, EngineCfg::default(), "w5-epub").unwrap();
    let turns = st2.turns_of(nb.id).unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[1].citations.len(), 1);
    assert_eq!(turns[1].citations[0].chunk_id, Some(top_id));
}

#[test]
fn rechunk_replaces_chunks_and_kills_citations_to_the_old_set() {
    let dir = temp_dir("rechunk");
    let mut st = CoreState::new(&dir, EngineCfg::default(), "w5-rechunk").unwrap();
    let nb = st.create_notebook("Rechunk", 1).unwrap();
    let src = st
        .add_source(nb.id, SourceKind::Text, "doc", "/doc.txt", 2)
        .unwrap();
    let mut emb = BowEmbedder { dim: 64 };

    // v1: the cat text. One long paragraph, one chunk.
    let v1 = "猫 は 魚 が 好き で 昼寝 を する 。 ".repeat(30);
    let made1 = st.set_source_text(src.id, &v1).unwrap();
    assert_eq!(made1.len(), 1);
    let old_id = made1[0].id;
    assert_eq!(st.index_source(src.id, &mut emb).unwrap(), 1);

    // v2: a different text on the same source.
    let v2 = "犬 は 公園 を 散歩 し て 走る 。 ".repeat(30);
    let made2 = st.set_source_text(src.id, &v2).unwrap();
    assert_eq!(made2.len(), 1);
    let new_id = made2[0].id;
    assert_ne!(old_id, new_id);
    assert_eq!(st.index_source(src.id, &mut emb).unwrap(), 1);

    // The old chunk is gone from memory and from a fresh state alike.
    assert!(st.chunk_by_id(old_id).is_none());
    let current = st.chunks_of(src.id);
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].id, new_id);
    drop(st);
    let mut st = CoreState::new(&dir, EngineCfg::default(), "w5-rechunk").unwrap();
    assert!(st.chunk_by_id(old_id).is_none());
    assert_eq!(st.chunks_of(src.id).len(), 1);

    // Search only ever sees the new set.
    let q = emb.embed("猫 魚").unwrap();
    let hits = st.search(nb.id, &q, 5);
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|(c, _)| c.id == new_id && c.text.contains("犬")));

    // Through the ask path: an answer citing the dead chunk is
    // rejected by the gate; the retry citing the live chunk is kept.
    let prepared = st.ask_prepare(nb.id, "犬 公園", &mut emb, 2).unwrap();
    let mut gen = QueueGenerator::new(
        vec![
            format!("猫は魚が好きです[#{old_id}]。"),
            format!("犬は公園を散歩します[#{new_id}]。"),
        ],
        0,
    );
    let answer = ask_generate_with_sink(&prepared, &mut gen, &mut |_| {}).unwrap();
    assert_eq!(gen.prompts.len(), 2, "the dead citation forced the one retry");
    assert_eq!(answer.citations.len(), 1);
    assert_eq!(answer.citations[0].chunk_id, Some(new_id));
}

#[test]
fn scope_keeps_the_other_notebooks_chunks_and_citations_out() {
    let dir = temp_dir("scope");
    let mut st = CoreState::new(&dir, EngineCfg::default(), "w5-scope").unwrap();
    let nb_a = st.create_notebook("A", 1).unwrap();
    let nb_b = st.create_notebook("B", 2).unwrap();
    let src_a = st
        .add_source(nb_a.id, SourceKind::Text, "cats", "/a/cats.txt", 3)
        .unwrap();
    let src_b = st
        .add_source(nb_b.id, SourceKind::Text, "dogs", "/b/dogs.txt", 4)
        .unwrap();
    let mut emb = BowEmbedder { dim: 64 };
    let cats = "猫 は 魚 が 好き で 昼寝 を する 。 ".repeat(30);
    let dogs = "犬 は 公園 を 散歩 し て 走る 。 ".repeat(30);
    st.set_source_text(src_a.id, &cats).unwrap();
    let b_chunks = st.set_source_text(src_b.id, &dogs).unwrap();
    let b_chunk_id = b_chunks[0].id;
    assert_eq!(st.index_source(src_a.id, &mut emb).unwrap(), 1);
    assert_eq!(st.index_source(src_b.id, &mut emb).unwrap(), 1);

    // The scope of A names A's paths only.
    let scope = st.scope_for(nb_a.id);
    assert!(scope.allows("/a/cats.txt"));
    assert!(!scope.allows("/b/dogs.txt"));
    assert_eq!(scope.len(), 1);

    // Even with B's own query vector, search inside A returns A's
    // chunks only.
    let q = emb.embed("犬 公園").unwrap();
    let hits = st.search(nb_a.id, &q, 5);
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|(c, _)| c.nb == nb_a.id && !c.text.contains("犬")));

    // Through the ask path: a generator that insists on citing B's
    // chunk never validates inside A, so the gate refuses.
    let mut gen = QueueGenerator::new(
        vec![
            format!("犬は公園を散歩します[#{b_chunk_id}]。"),
            format!("犬は公園を散歩します[#{b_chunk_id}]。"),
        ],
        0,
    );
    let turn = st.ask(nb_a.id, "犬 公園", &mut emb, &mut gen, 2, 10).unwrap();
    assert_eq!(gen.prompts.len(), 2);
    assert_eq!(turn.text, "資料にありません。");
    assert!(turn.citations.is_empty());
}
