#!/usr/bin/env python3
"""Regenerate every fixture in this directory, deterministically.

Source design: notebook-design annexes corpus-a.md (categories 1-9)
and corpus-a2.md (categories 10-16), EXTRACTION face only (the
chunk_text and parse_script faces belong to other crates). Category
11 (trailing-whitespace-only difference) has no extraction face in
the annex ("対応物なし": extraction normalizes it away), so there
are no category-11 fixtures here.

Usage:  python3 make_fixtures.py
Output: txt/, html/, epub/, undefined/ under this directory.
Zip entries use a fixed 1980 timestamp so regeneration is
byte-stable.
"""

import io
import os
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))

P1 = "これは最初の段落です。約百五十文字程度の内容を含みます。段落の境界は空行で検出されます。テキスト抽出の固定フィクスチャとして使う文章です。追加の文をここに置きます。"
P2 = "これは2番目の段落です。同様に約百五十文字程度の内容を含みます。中間の空行や末尾の空行が抽出結果にどう影響するかを見るための文章です。追加の文をここに置きます。"
PA = "段落Aの内容です。約百五十文字程度で構成されています。この段落は最初のチャンクになります。空行の扱いを見るための文章です。追加の文をここに置きます。さらに続けます。"
PB = "段落Bの内容です。約百五十文字程度で構成されています。中間の複数空行は単一の段落境界として扱われるかを見る文章です。追加の文をここに置きます。さらに続けます。"
PC = "段落Cの内容です。約百五十文字程度で構成されています。末尾の単一空行は次の段落がないため無視されるかを見る文章です。追加の文をここに置きます。さらに続けます。"
LONG_SENTENCE = "これは非常に長い入力のテストです。"
LONG_TEXT = LONG_SENTENCE * 600  # 10,200 chars
B380 = "あ" * 380
B60 = "い" * 60
B380C = "う" * 380


def w(subdir, name, data):
    if isinstance(data, str):
        data = data.encode("utf-8")
    path = os.path.join(HERE, subdir, name)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)
    print(f"{subdir}/{name}: {len(data)} bytes")


def sjis(s):
    return s.encode("shift_jis")


def zip_bytes(entries):
    """entries: list of (name, bytes). Stored, fixed timestamps."""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_STORED) as z:
        for name, data in entries:
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_STORED
            z.writestr(info, data)
    return buf.getvalue()


CONTAINER = (
    '<?xml version="1.0"?>'
    '<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container">'
    "<rootfiles>"
    '<rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>'
    "</rootfiles></container>"
)


def epub(chapters, spine=None, manifest_extra="", extra_entries=()):
    """chapters: list of (id, body_markup). Builds a minimal EPUB."""
    if spine is None:
        spine = [cid for cid, _ in chapters]
    items = "".join(
        f'<item id="{cid}" href="{cid}.xhtml" media-type="application/xhtml+xml"/>'
        for cid, _ in chapters
    )
    items += manifest_extra
    refs = "".join(f'<itemref idref="{r}"/>' for r in spine)
    opf = (
        '<?xml version="1.0"?>'
        '<package xmlns="http://www.idpf.org/2007/opf">'
        f"<manifest>{items}</manifest><spine>{refs}</spine></package>"
    )
    entries = [
        ("META-INF/container.xml", CONTAINER.encode()),
        ("OEBPS/content.opf", opf.encode()),
    ]
    for cid, body in chapters:
        xhtml = (
            '<?xml version="1.0" encoding="utf-8"?>'
            '<html xmlns="http://www.w3.org/1999/xhtml">'
            f"<body>{body}</body></html>"
        )
        entries.append((f"OEBPS/{cid}.xhtml", xhtml.encode()))
    entries.extend(extra_entries)
    return zip_bytes(entries)


def corrupt(zip_data, marker):
    """Flip one byte inside the stored data carrying `marker`
    (guarantees a CRC mismatch when that entry is read)."""
    i = zip_data.find(marker)
    assert i >= 0, marker
    b = bytearray(zip_data)
    b[i] ^= 0xFF
    return bytes(b)


def main():
    # ---- category 1: 0 lines ----
    w("txt", "00_empty.bin", b"")
    w("html", "00_empty.html", b"")
    w("epub", "00_empty.epub", zip_bytes([]))

    # ---- category 2: 1 line ----
    w("txt", "01_single_line_utf8.bin", "単一行のテキストです")
    w("txt", "01_single_line_sjis.bin", sjis("単一行のテキストです"))
    w("html", "01_single_line.html", "<html><body>単一行のHTMLです</body></html>")
    w("epub", "01_single_line.epub", epub([("c1", "<p>単一行のEPUBです</p>")]))

    # ---- category 3: all empty ----
    w("txt", "02_all_empty_utf8.bin", "\n\n\n\n\n")
    w("txt", "02_all_empty_sjis.bin", sjis("\n\n\n\n\n"))
    w("html", "02_all_empty.html", "<html><body><p></p><p></p></body></html>")
    w("epub", "02_all_empty.epub", epub([("c1", "<p></p>"), ("c2", "<p></p>")]))

    # ---- category 4: leading blank lines ----
    lead = "\n\n最初の段落です。\n\n2番目の段落です。"
    w("txt", "03_leading_empty_utf8.bin", lead)
    w("txt", "03_leading_empty_sjis.bin", sjis(lead))
    w("html", "03_leading_empty.html",
      "<html><body><p></p><p></p><p>最初の段落です。</p><p>2番目の段落です。</p></body></html>")
    w("epub", "03_leading_empty.epub",
      epub([("c1", "<p></p><p></p>"), ("c2", "<p>実質のある段落です。</p>")]))

    # ---- category 5: blank lines in the middle ----
    middle = PA + "\n\n\n\n" + PB + "\n\n" + PC
    w("txt", "04_middle_empty_utf8.bin", middle)
    w("txt", "04_middle_empty_sjis.bin", sjis(middle))
    w("html", "04_middle_empty.html",
      f"<p>{PA}</p><p></p><p></p><p>{PB}</p><p></p><p>{PC}</p>")
    w("epub", "04_middle_empty.epub",
      epub([("c1", f"<p>{PA}</p><p></p><p></p><p>{PB}</p><p></p><p>{PC}</p>")]))

    # ---- category 6: trailing blank lines ----
    trail = P1 + "\n\n" + P2 + "\n\n\n\n"
    w("txt", "05_trailing_empty_utf8.bin", trail)
    w("txt", "05_trailing_empty_sjis.bin", sjis(trail))
    w("html", "05_trailing_empty.html", "<p>段落1</p><p>段落2</p><p></p><p></p><p></p>")
    w("epub", "05_trailing_empty.epub",
      epub([("c1", "<p>実質のある段落です。</p>"), ("c2", "<p></p><p></p>")]))

    # ---- category 7: whitespace-only lines ----
    ws = P1 + "\n" + "   " + "\n" + "\t\t" + "\n" + " \t " + "\n" + P2
    w("txt", "06_whitespace_only_utf8.bin", ws)
    w("txt", "06_whitespace_only_sjis.bin", sjis(ws))
    w("html", "06_whitespace_only.html",
      "<p>段落1</p><p>   </p><p>\t\t</p><p> \t </p><p>段落2</p>")
    w("epub", "06_whitespace_only.epub",
      epub([("c1", "<p>段落1</p><p>   </p><p>\t\t</p><p> \t </p><p>段落2</p>")]))

    # ---- category 8: very long input (~10,000 chars) ----
    w("txt", "07_very_long_utf8.bin", LONG_TEXT)
    w("txt", "07_very_long_sjis.bin", sjis(LONG_TEXT))
    w("html", "07_very_long.html", f"<html><body><p>{LONG_TEXT}</p></body></html>")
    w("epub", "07_very_long.epub", epub([("c1", f"<p>{LONG_TEXT}</p>")]))

    # ---- category 9: totals crossing the ~400-char boundary ----
    bound = B380 + "\n\n" + B60 + "\n\n" + B380C
    w("txt", "08_boundary_cross_utf8.bin", bound)
    w("txt", "08_boundary_cross_sjis.bin", sjis(bound))
    w("html", "08_boundary_cross.html", f"<p>{B380}</p><p>{B60}</p><p>{B380C}</p>")
    w("epub", "08_boundary_cross.epub",
      epub([("c1", f"<p>{B380}</p>"), ("c2", f"<p>{B60}</p>"), ("c3", f"<p>{B380C}</p>")]))

    # ---- category 10: duplicate lines ----
    block = "同一バイト列のテストです。これは重複行カテゴリのフィクスチャです。\n" * 6
    w("txt", "10_dup_lines.bin", (block + block).encode())
    w("html", "10_dup_lines.html", "<p>同一文</p><p>同一文</p><p>同一文</p><p>同一文</p>")
    w("epub", "10_dup_lines.epub",
      epub([("c1", "<p>同一の<ruby>文章<rt>ぶんしょう</rt></ruby>です。</p>")],
           spine=["c1", "c1"]))

    # ---- category 12: failing element first ----
    w("txt", "12_fail_first.bin",
      b"\xff\xfe\x00\x00" + "正常なUTF-8テキストです。".encode())
    w("html", "12_fail_first.html",
      "<broken><html><body><p>正常なHTMLです</p></body></html>")
    w("epub", "12_fail_first.epub",
      epub([("c1", "<p>正常な章です。</p>")],
           spine=["missing", "c1"],
           manifest_extra='<item id="missing" href="missing.xhtml" media-type="application/xhtml+xml"/>'))

    # ---- category 13: failing element in the middle ----
    w("txt", "13_fail_middle.bin",
      "前半は正常なテキストです。".encode() + b"\xed\xa0\x80" + "後半も正常なテキストです。".encode())
    w("html", "13_fail_middle.html",
      '<p>前半は正常です。</p><script>var x = "未閉じ";<p>後半も正常です。</p>')
    mid = epub([("c1", "<p>第1章です。</p>"),
                ("c2", "<p>CORRUPTME13 第2章です。</p>"),
                ("c3", "<p>第3章です。</p>")])
    w("epub", "13_fail_middle.epub", corrupt(mid, b"CORRUPTME13"))

    # ---- category 14: failing element last ----
    w("txt", "14_fail_last.bin",
      "正常なUTF-8テキストです".encode() + b"\xe3\x81")
    w("html", "14_fail_last.html",
      "<html><body><p>正常なHTMLです</p></body></html><div>")
    last = epub([("c1", "<p>正常な章です。</p>"),
                 ("broken", "<p>CORRUPTME14 壊れた章です。</p>")])
    w("epub", "14_fail_last.epub", corrupt(last, b"CORRUPTME14"))

    # ---- category 15: failing elements everywhere ----
    w("txt", "15_fail_all.bin", b"\xff\xfe" * 32)
    w("html", "15_fail_all.html", "<broken><nested<attr=>>")
    w("epub", "15_fail_all.epub", zip_bytes([("dummy.txt", b"no epub structure here")]))

    # ---- category 16: undefined verdicts (separate bucket) ----
    w("undefined", "txt_empty.bin", b"")
    w("undefined", "html_no_visible.html", "<html></html>")
    w("undefined", "epub_nav_only.epub",
      epub([("nav", "<nav></nav>")]))


if __name__ == "__main__":
    main()
