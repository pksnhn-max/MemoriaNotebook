#!/usr/bin/env python3
"""Deterministic generator for the W3 fixed-corpus fixtures.

Regenerates every file under ledger/, emb/, wav/ (including
wav/rejects/) next to this script. Run:

    python3 generate_fixtures.py

Fixture shapes come from the corpus-b1 (ledger/sidecar) and corpus-b2
(WAV) design annexes, re-expressed in the REAL record formats of
notebook-core:

- ledger/*.jsonl lines are ChunkRec JSON records
  (id/source_id/nb/ord/rev/text); deleted ledgers are bare u64 lines.
  (The annex's {"id","text","embedding_id"} schema does not exist in
  the real code.)
- emb/*.bin records are (u64 LE chunk_id, u32 LE dim, f32[dim] LE),
  the emb_chunks.bin wire format.
- wav/*.wav are canonical 44-byte-header WAV files unless the case is
  specifically about a broken header.

Everything is byte-deterministic except the 1 kHz sine samples, which
are rounded to i16 (stable across platforms in practice; the tests
never compare fixture bytes against a golden, only live vs oracle on
the same files).
"""

import json
import math
import os
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def write(rel, data):
    path = os.path.join(HERE, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    mode = "wb" if isinstance(data, bytes) else "w"
    with open(path, mode) as f:
        f.write(data)
    print("wrote", rel, len(data))


# ------------------------------------------------------------------
# ledger (JSONL)
# ------------------------------------------------------------------

def rec(cid, text, nb=1, rev=1, ord_=0, source_id=1, extra=None):
    obj = {
        "id": cid,
        "source_id": source_id,
        "nb": nb,
        "ord": ord_,
        "rev": rev,
        "text": text,
    }
    if extra:
        obj.update(extra)
    return json.dumps(obj, separators=(",", ":"), ensure_ascii=False) + "\n"


def ledger_fixtures():
    write("ledger/00_empty.jsonl", "")
    write("ledger/01_one_valid.jsonl", rec(1, "hello"))
    write("ledger/02_all_blank.jsonl", "\n\n\n")
    write("ledger/10_blank_head.jsonl", "\n" + rec(1, "a") + rec(2, "b", ord_=1))
    write("ledger/11_blank_middle.jsonl", rec(1, "a") + "\n" + rec(2, "b", ord_=1))
    write("ledger/12_blank_tail.jsonl", rec(1, "a") + rec(2, "b", ord_=1) + "\n")
    write(
        "ledger/13_whitespace_only.jsonl",
        "   \n\t\n  \t  \n" + rec(1, "ok"),
    )
    write(
        "ledger/14_broken_json.jsonl",
        rec(1, "a") + "{not valid json\n" + rec(2, "b", ord_=1),
    )
    write(
        "ledger/15_wrong_type.jsonl",
        rec(1, "a") + '["array",1,2]\n' + rec(3, "c", ord_=2),
    )
    write(
        "ledger/16_mixed_garbage.jsonl",
        "\n{broken\n   \n" + rec(1, "ok") + "[1,2]\n\n" + rec(2, "ok2", ord_=1),
    )
    write(
        "ledger/20_very_long_line.jsonl",
        rec(1, "x" * 1_000_000) + rec(2, "short", ord_=1),
    )
    write(
        "ledger/21_long_middle.jsonl",
        rec(1, "a") + rec(2, "y" * 2_000_000, ord_=1) + rec(3, "c", ord_=2),
    )
    write(
        "ledger/22_long_tail.jsonl",
        rec(1, "a") + rec(2, "z" * 500_000, ord_=1),
    )
    write(
        "ledger/30_dup_id.jsonl",
        rec(1, "a") + rec(1, "a-v2", rev=2, ord_=1) + rec(2, "b", ord_=2),
    )
    write(
        "ledger/31_dup_nonadjacent.jsonl",
        rec(1, "a") + rec(2, "b", ord_=1) + rec(1, "a-v2", rev=2, ord_=2),
    )
    # Tombstones live in deleted.jsonl as bare notebook ids (the real
    # format); 32/33/71 pair a chunks file with a deleted file.
    write(
        "ledger/32_chunks.jsonl",
        rec(1, "a", nb=1) + rec(2, "b", nb=1, ord_=1) + rec(3, "c", nb=2, ord_=2),
    )
    write("ledger/32_deleted.jsonl", "1\n")
    write("ledger/33_deleted.jsonl", "1\n1\n")
    write(
        "ledger/40_fail_head.jsonl",
        "{broken json\n" + rec(1, "ok") + rec(2, "ok2", ord_=1),
    )
    write(
        "ledger/41_fail_middle.jsonl",
        rec(1, "ok") + "{broken\n" + rec(3, "ok3", ord_=2),
    )
    write(
        "ledger/42_fail_tail.jsonl",
        rec(1, "ok") + rec(2, "ok2", ord_=1) + "{broken",
    )
    write("ledger/43_all_fail.jsonl", '{broken\n{also broken\n["array"]\n123\n')
    write(
        "ledger/50_partial_tail_line.jsonl",
        rec(1, "ok")
        + '{"id":2,"source_id":1,"nb":1,"ord":1,"rev":1,"text":"incomplete',
    )
    write(
        "ledger/51_partial_mid_line.jsonl",
        rec(1, "ok") + '{"id":2,"source_id":1,"nb":1,"ord',
    )
    write(
        "ledger/52_chunks.jsonl",
        rec(100, "a") + rec(101, "b", ord_=1) + rec(102, "c", ord_=2),
    )
    write(
        "ledger/60_resume_twice.jsonl",
        "".join(rec(i, chr(ord("a") + i - 1), ord_=i - 1) for i in range(1, 6)),
    )
    write(
        "ledger/61_resume_with_garbage.jsonl",
        rec(1, "a") + "{garbage\n" + rec(2, "b", ord_=1) + "\n" + rec(3, "c", ord_=2),
    )
    write(
        "ledger/70_chunks.jsonl",
        rec(1, "a") + rec(2, "b", ord_=1) + rec(999, "phantom", ord_=2),
    )
    write(
        "ledger/71_chunks.jsonl",
        rec(1, "a", nb=1) + rec(2, "b", nb=1, ord_=1),
    )
    write("ledger/71_deleted.jsonl", "1\n")
    # Category 8 (skip sets): read_jsonl has no skip-set parameter, so
    # these are plain files; the skip dimension is exercised on the
    # sidecar side via load_embeddings' current-chunk selection.
    write("ledger/80_skip_empty.jsonl", rec(1, "a") + rec(2, "b", ord_=1))
    write("ledger/81_skip_all.jsonl", rec(1, "a") + rec(2, "b", ord_=1))
    write(
        "ledger/82_skip_overlap_fail.jsonl",
        rec(1, "ok") + "{broken\n" + rec(3, "ok3", ord_=2),
    )
    # Undefined bucket: a line missing required fields, a line with an
    # unknown extra field. (Real serde semantics are in fact defined —
    # skip / accept — but the annex buckets these as undefined, so the
    # test asserts only that the reader does not panic.)
    write(
        "ledger/83_undefined_bucket.jsonl",
        rec(1, "a")
        + '{"id":2,"source_id":1}\n'
        + rec(3, "c", ord_=2, extra={"extra": True}),
    )


# ------------------------------------------------------------------
# emb (sidecar)
# ------------------------------------------------------------------

def erec(cid, floats):
    return struct.pack("<QI", cid, len(floats)) + struct.pack(
        "<%df" % len(floats), *floats
    )


def ehdr(cid, dim):
    return struct.pack("<QI", cid, dim)


def efloats(n, start=1.0):
    return [float(start + i) for i in range(n)]


def emb_fixtures():
    v4 = [1.0, 2.0, 3.0, 4.0]
    w4 = [5.0, 6.0, 7.0, 8.0]
    write("emb/00_empty.bin", b"")
    write("emb/01_one_vec.bin", erec(100, [0.1, 0.2, 0.3, 0.4]))
    write("emb/02_no_corresponding_ledger.bin", erec(999, [1.0, 2.0, 3.0]))
    write("emb/20_dim_mismatch_short.bin", ehdr(10, 1000) + struct.pack("<10f", *efloats(10)))
    write(
        "emb/21_dim_mismatch_long.bin",
        ehdr(11, 3) + struct.pack("<10f", *efloats(10)) + erec(12, [7.0, 8.0]),
    )
    write("emb/22_truncated_tail.bin", ehdr(20, 4) + struct.pack("<2f", 1.0, 2.0))
    write(
        "emb/23_truncated_middle.bin",
        erec(30, v4) + ehdr(31, 4) + struct.pack("<2f", 1.0, 2.0) + erec(32, [5.0, 6.0]),
    )
    write("emb/30_dup_chunk_id.bin", erec(10, v4) + erec(10, w4))
    write("emb/31_dup_nonadjacent.bin", erec(10, v4) + erec(11, [9.0, 10.0, 11.0, 12.0]) + erec(10, w4))
    write(
        "emb/40_fail_head.bin",
        ehdr(10, 1000) + struct.pack("<10f", *efloats(10)) + erec(11, v4) + erec(12, w4),
    )
    write(
        "emb/41_fail_middle.bin",
        erec(10, v4) + ehdr(11, 1000) + struct.pack("<10f", *efloats(10)) + erec(12, w4),
    )
    write(
        "emb/42_fail_tail.bin",
        erec(10, v4) + erec(11, w4) + ehdr(12, 1000) + struct.pack("<10f", *efloats(10)),
    )
    write(
        "emb/43_all_fail.bin",
        b"".join(ehdr(i, 1000) + struct.pack("<10f", *efloats(10)) for i in (10, 11, 12)),
    )
    write("emb/50_truncated_tail_record.bin", erec(10, v4) + ehdr(11, 4) + struct.pack("<2f", 1.0, 2.0))
    write(
        "emb/51_truncated_mid_record.bin",
        erec(10, v4) + ehdr(11, 4) + struct.pack("<2f", 1.0, 2.0) + erec(12, w4),
    )
    write("emb/52_partial.bin", erec(100, v4) + erec(101, w4))
    write("emb/52_full.bin", erec(100, v4) + erec(101, w4) + erec(102, [9.0, 10.0, 11.0, 12.0]))
    write("emb/60_resume_twice.bin", b"".join(erec(i, v4) for i in (10, 11, 12, 13, 14)))
    write(
        "emb/61_resume_with_truncation.bin",
        erec(10, v4) + ehdr(11, 4) + struct.pack("<2f", 1.0, 2.0) + erec(12, w4) + erec(13, v4),
    )
    write("emb/70_extra_vectors.bin", erec(1, v4) + erec(2, w4) + erec(999, v4))
    write("emb/71_vector_for_deleted.bin", erec(1, v4) + erec(2, w4))
    write("emb/80_skip_empty.bin", erec(10, v4) + erec(11, w4))
    write("emb/81_skip_all.bin", erec(10, v4) + erec(11, w4))
    write(
        "emb/82_skip_overlap_corrupt.bin",
        erec(10, v4) + ehdr(11, 1000) + struct.pack("<10f", *efloats(10)) + erec(12, w4),
    )
    write("emb/83_undefined_bucket.bin", erec(10, v4) + erec(11, []) + erec(12, w4))


# ------------------------------------------------------------------
# wav
# ------------------------------------------------------------------

def wav_bytes(rate, ch, bits, data, fmt_tag=1, fmt_size=16, data_size=None,
              riff_id=b"RIFF", wave_id=b"WAVE", with_data_chunk=True):
    bytes_per_sample = bits // 8
    byte_rate = rate * ch * bytes_per_sample
    block_align = ch * bytes_per_sample
    fmt = struct.pack("<HHIIHH", fmt_tag, ch, rate, byte_rate, block_align, bits)
    if fmt_size > 16:
        fmt += b"\x00" * (fmt_size - 16)
    body = b"fmt " + struct.pack("<I", fmt_size) + fmt
    if with_data_chunk:
        ds = len(data) if data_size is None else data_size
        body += b"data" + struct.pack("<I", ds) + data
    return riff_id + struct.pack("<I", 4 + len(body)) + wave_id + body


def silence(ms, rate=24000):
    n = rate * ms // 1000
    return b"\x00\x00" * n


def sine_500ms_plus_tail(tail_ms):
    samples = []
    for n in range(12000):  # 500 ms at 24 kHz
        v = int(8000 * math.sin(2 * math.pi * 1000 * n / 24000))
        samples.append(v)
    body = struct.pack("<%dh" % len(samples), *samples)
    return body + silence(tail_ms)


def wav_fixtures():
    s50 = silence(50)
    s100 = silence(100)
    s200 = silence(200)
    write("wav/silence_50ms.wav", wav_bytes(24000, 1, 16, s50))
    write("wav/silence_100ms.wav", wav_bytes(24000, 1, 16, s100))
    write("wav/silence_200ms.wav", wav_bytes(24000, 1, 16, s200))
    write("wav/single_100ms.wav", wav_bytes(24000, 1, 16, s100))
    write("wav/good_100ms.wav", wav_bytes(24000, 1, 16, s100))
    write("wav/sr_24k.wav", wav_bytes(24000, 1, 16, s100))
    write("wav/zero_data.wav", wav_bytes(24000, 1, 16, b""))
    write("wav/tail_sil_100ms.wav", wav_bytes(24000, 1, 16, sine_500ms_plus_tail(100)))
    write("wav/tail_sil_200ms.wav", wav_bytes(24000, 1, 16, sine_500ms_plus_tail(200)))
    write("wav/tail_sil_300ms.wav", wav_bytes(24000, 1, 16, sine_500ms_plus_tail(300)))
    write("wav/long_10min.wav", wav_bytes(24000, 1, 16, silence(600_000)))
    # Undefined bucket (rejects/): no fixed expectation, no-panic only.
    write("wav/rejects/header_only.wav",
          wav_bytes(24000, 1, 16, b"", with_data_chunk=False))
    write("wav/rejects/sr_48k.wav", wav_bytes(48000, 1, 16, silence(100, 48000)))
    write("wav/rejects/sr_8k.wav", wav_bytes(8000, 1, 16, silence(100, 8000)))
    write("wav/rejects/ch2.wav", wav_bytes(24000, 2, 16, b"\x00\x00" * 4800))
    write("wav/rejects/bits24.wav", wav_bytes(24000, 1, 24, b"\x00" * 7200))
    write("wav/rejects/bits32float.wav", wav_bytes(24000, 1, 32, b"\x00" * 9600, fmt_tag=3))
    write("wav/rejects/bad_riff_id.wav",
          wav_bytes(24000, 1, 16, s100, riff_id=b"RIFX"))
    write("wav/rejects/bad_wave_id.wav",
          wav_bytes(24000, 1, 16, s100, wave_id=b"XXXX"))
    write("wav/rejects/bad_fmt_size.wav",
          wav_bytes(24000, 1, 16, s100, fmt_size=18))
    write("wav/rejects/bad_data_size.wav",
          wav_bytes(24000, 1, 16, s100, data_size=999999))
    write("wav/rejects/truncated.wav",
          wav_bytes(24000, 1, 16, s100)[: 44 + 2400])
    write("wav/rejects/not_riff.mp3", b"\xff\xfb" + b"\x00" * 98)
    write("wav/rejects/not_riff.raw", b"\x00" * 4800)
    write("wav/rejects/not_riff.txt", b"hello")


if __name__ == "__main__":
    os.chdir(HERE)
    ledger_fixtures()
    emb_fixtures()
    wav_fixtures()
    print("done", file=sys.stderr)
