# Fixed corpus — extraction face

Fixed-layer fixtures (testing-manual §2) for notebook-extract,
built from the design annexes `corpus-a.md` (categories 1–9) and
`corpus-a2.md` (categories 10–16), **extraction face only**
（資料バイト： `extract_txt` / `html_to_text` / `extract_epub`).
The chunk_text and parse_script faces of the same categories live
with their own crates.

## Regeneration

```
python3 make_fixtures.py
```

Deterministic: zip entries carry a fixed 1980 timestamp, and
re-running the script reproduces every file byte-for-byte
(verified by checksum). Edit the script, not the outputs.

## Layout

- `txt/` — inputs for `extract_txt` (numbered by annex category:
  00 empty, 01 single line, 02 all empty, 03 leading blanks,
  04 middle blanks, 05 trailing blanks, 06 whitespace-only lines,
  07 very long, 08 boundary-crossing totals, 10 duplicate lines,
  12/13/14 failing element first/middle/last, 15 all failing).
- `html/` — same category numbers, inputs for `html_to_text`.
- `epub/` — same category numbers, inputs for `extract_epub`.
  13/14 carry one byte flipped inside a stored chapter entry, so
  reading that entry fails its CRC check.
- `undefined/` — category 16, the separate bucket for inputs whose
  correct verdict is undefined (testing-manual §2). They are still
  pinned differentially (live must equal oracle), but no
  correctness claim is made about the outcome itself.

Category 11 (lines differing only in trailing whitespace) has **no
extraction face** in the annex — extraction normalizes it away
before it can be observed — so there are deliberately no
category-11 files here.

## How the tests use them

`src/fixture_tests.rs` feeds every file to both the live
implementation and the frozen oracle (`src/oracle.rs`) and asserts
the outcomes are identical. The fixtures pin *actual* behaviour
(characterization); where the annex prose claims a different
behaviour than the pinned one, the record book for brief W4 lists
the divergence.
