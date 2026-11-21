//! ORACLE (differential testing, testing-manual §2): verbatim copies of
//! `percent_encode` / `percent_decode` / `hex` / `hf_resolve_url` from
//! `lib.rs`, frozen 2026-11-21 from the tree at repo HEAD 3118d042.
//! NEVER fix this module — bugs included.
//!
//! Only deviation: `hex` widened to `pub` (body unchanged) so the
//! differential tests can call it.

#![allow(dead_code)]

pub fn percent_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let h = hex(b[i + 1]);
                let l = hex(b[i + 2]);
                if let (Some(h), Some(l)) = (h, l) {
                    out.push(h * 16 + l);
                    i += 3;
                } else {
                    out.push(b[i]);
                    i += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn hf_resolve_url(repo: &str, file: &str) -> String {
    // Encode each path segment (file names may carry spaces etc.);
    // '/' separators between segments stay literal.
    let enc = |s: &str| {
        s.split('/')
            .map(|seg| percent_encode(seg.as_bytes()))
            .collect::<Vec<_>>()
            .join("/")
    };
    format!(
        "https://huggingface.co/{}/resolve/main/{}",
        enc(repo),
        enc(file)
    )
}

