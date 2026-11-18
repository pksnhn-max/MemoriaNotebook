//! Network seat: model-file download, web search, and page fetch.
//! Everything the app pulls from outside comes through here, behind
//! plain functions — no other crate talks to the network.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum NetError {
    Http(String),
    Io(std::io::Error),
    TooLarge,
    Parse(&'static str),
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::Http(m) => write!(f, "http: {m}"),
            NetError::Io(e) => write!(f, "io: {e}"),
            NetError::TooLarge => write!(f, "body too large"),
            NetError::Parse(what) => write!(f, "parse: {what}"),
        }
    }
}
impl std::error::Error for NetError {}
impl From<std::io::Error> for NetError {
    fn from(e: std::io::Error) -> Self {
        NetError::Io(e)
    }
}

const UA: &str = "Mozilla/5.0 (Android 14; MemoriaNotebook) AppleWebKit/537.36";

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(20))
        .timeout_read(std::time::Duration::from_secs(120))
        .user_agent(UA)
        .build()
}

/// Download `url` to `dest` atomically (via a `.part` sibling that is
/// renamed only on success). `progress(done, total_or_0)` is called as
/// bytes land. Returns the byte count.
pub fn download_to_file(
    url: &str,
    dest: &Path,
    mut progress: impl FnMut(u64, u64),
) -> Result<u64, NetError> {
    let resp = agent()
        .get(url)
        .call()
        .map_err(|e| NetError::Http(e.to_string()))?;
    let total: u64 = resp
        .header("Content-Length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut part = dest.to_path_buf();
    part.set_extension("part");
    let mut file = std::fs::File::create(&part)?;
    let mut reader = resp.into_reader();
    let mut buf = [0u8; 64 * 1024];
    let mut done: u64 = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        progress(done, total);
    }
    file.sync_all()?;
    drop(file);
    std::fs::rename(&part, dest)?;
    Ok(done)
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Default SearXNG instance for the fallback provider. The instance
/// is a replaceable part: settings can point at any SearXNG host.
pub const DEFAULT_SEARX_INSTANCE: &str = "https://searx.be";

/// Web search (no key). Provider order: DuckDuckGo HTML first; when
/// it yields nothing (bot walls refuse datacenter clients), fall back
/// to a SearXNG instance's JSON API. Returns up to `n` hits with
/// redirect wrappers unwrapped to the real URLs.
pub fn search_web(query: &str, n: usize) -> Result<Vec<SearchHit>, NetError> {
    match search_web_ddg(query, n) {
        Ok(hits) if !hits.is_empty() => Ok(hits),
        _ => search_web_searx(DEFAULT_SEARX_INSTANCE, query, n),
    }
}

pub fn search_web_ddg(query: &str, n: usize) -> Result<Vec<SearchHit>, NetError> {
    let url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        percent_encode(query.as_bytes())
    );
    let body = get_text(&url, 2 * 1024 * 1024)?;
    Ok(parse_ddg_html(&body, n))
}

pub fn search_web_searx(
    instance: &str,
    query: &str,
    n: usize,
) -> Result<Vec<SearchHit>, NetError> {
    let url = format!(
        "{}/search?q={}&format=json",
        instance.trim_end_matches('/'),
        percent_encode(query.as_bytes())
    );
    let body = get_text(&url, 4 * 1024 * 1024)?;
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|_| NetError::Parse("searx json"))?;
    let mut hits = Vec::new();
    if let Some(results) = v.get("results").and_then(|r| r.as_array()) {
        for r in results.iter().take(n) {
            let get = |k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let url = get("url");
            if !url.is_empty() {
                hits.push(SearchHit {
                    title: get("title"),
                    url,
                    snippet: get("content"),
                });
            }
        }
    }
    Ok(hits)
}

fn get_text(url: &str, cap: usize) -> Result<String, NetError> {
    let resp = agent()
        .get(url)
        .call()
        .map_err(|e| NetError::Http(e.to_string()))?;
    let mut reader = resp.into_reader();
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 32 * 1024];
    loop {
        let m = reader.read(&mut chunk)?;
        if m == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..m]);
        if buf.len() > cap {
            return Err(NetError::TooLarge);
        }
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Fetch a page and return its readable text (HTML stripped).
pub fn fetch_page_text(url: &str) -> Result<String, NetError> {
    let body = get_text(url, 4 * 1024 * 1024)?;
    let text = notebook_extract::html_to_text(&body);
    if text.trim().is_empty() {
        return Err(NetError::Parse("empty page text"));
    }
    Ok(text)
}

fn parse_ddg_html(html: &str, n: usize) -> Vec<SearchHit> {
    // Result anchors carry class "result__a" and an href with a uddg
    // parameter holding the real URL; snippets sit in "result__snippet".
    let mut hits = Vec::new();
    let mut rest = html;
    while hits.len() < n {
        let Some(a) = rest.find("result__a") else { break };
        rest = &rest[a..];
        let Some(href_start) = rest[..rest.len().min(2000)].find("href=\"") else {
            break;
        };
        let after = &rest[href_start + 6..];
        let Some(href_end) = after.find('"') else { break };
        let href = &after[..href_end];
        let Some(tag_end) = after.find('>') else { break };
        let title_html = &after[tag_end + 1..];
        let title_end = title_html.find("</a>").unwrap_or(0);
        let title = notebook_extract::html_to_text(&title_html[..title_end]);
        // Snippet: the next result__snippet after this anchor.
        let tail = &after[tag_end..];
        let snippet = match tail.find("result__snippet") {
            Some(s) => {
                let seg = &tail[s..];
                match seg.find('>') {
                    Some(gt) => {
                        let raw = &seg[gt + 1..];
                        let e = raw.find("</a>").unwrap_or(raw.len().min(400));
                        notebook_extract::html_to_text(&raw[..e])
                    }
                    None => String::new(),
                }
            }
            None => String::new(),
        };
        let real = unwrap_uddg(href);
        if !real.is_empty() {
            hits.push(SearchHit {
                title: title.trim().to_string(),
                url: real,
                snippet: snippet.trim().to_string(),
            });
        }
        rest = tail;
    }
    hits
}

fn unwrap_uddg(href: &str) -> String {
    if let Some(pos) = href.find("uddg=") {
        let v = &href[pos + 5..];
        let v = v.split('&').next().unwrap_or(v);
        return percent_decode(v);
    }
    if href.starts_with("http") {
        return href.to_string();
    }
    String::new()
}

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

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn hf_resolve_url(repo: &str, file: &str) -> String {
    format!("https://huggingface.co/{repo}/resolve/main/{file}")
}

/// Search Hugging Face for GGUF model repos matching `query`.
pub fn hf_search_models(query: &str) -> Result<Vec<String>, NetError> {
    let url = format!(
        "https://huggingface.co/api/models?search={}&filter=gguf&limit=30&sort=downloads&direction=-1",
        percent_encode(query.as_bytes())
    );
    let body = get_text(&url, 4 * 1024 * 1024)?;
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|_| NetError::Parse("hf search json"))?;
    let mut out = Vec::new();
    if let Some(arr) = v.as_array() {
        for m in arr {
            if let Some(id) = m.get("id").and_then(|x| x.as_str()) {
                out.push(id.to_string());
            }
        }
    }
    Ok(out)
}

/// List the `.gguf` files of a Hugging Face repo with their sizes.
pub fn hf_model_files(repo: &str) -> Result<Vec<(String, u64)>, NetError> {
    let url = format!("https://huggingface.co/api/models/{repo}?blobs=true");
    let body = get_text(&url, 8 * 1024 * 1024)?;
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|_| NetError::Parse("hf repo json"))?;
    let mut out = Vec::new();
    if let Some(sibs) = v.get("siblings").and_then(|s| s.as_array()) {
        for s in sibs {
            if let Some(name) = s.get("rfilename").and_then(|x| x.as_str()) {
                if name.to_ascii_lowercase().ends_with(".gguf") {
                    let size = s.get("size").and_then(|x| x.as_u64()).unwrap_or(0);
                    out.push((name.to_string(), size));
                }
            }
        }
    }
    Ok(out)
}

pub fn models_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("models")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_round_trip() {
        let s = "天満川 水位 & 100%";
        assert_eq!(percent_decode(&percent_encode(s.as_bytes())), s);
    }

    #[test]
    fn ddg_parse_unwraps() {
        let html = r##"<a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage&amp;rut=x">Example Title</a><a class="result__snippet" href="#">A snippet here.</a>"##;
        let hits = parse_ddg_html(html, 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://example.com/page");
        assert_eq!(hits[0].title, "Example Title");
        assert!(hits[0].snippet.contains("snippet"));
    }

    #[test]
    fn searx_parse() {
        let body = r#"{"results":[{"title":"天満川の水位","url":"https://example.com/tenma","content":"水位の観測記録"}]}"#;
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        let results = v.get("results").unwrap().as_array().unwrap();
        assert_eq!(results.len(), 1);
    }

    /// Transport checks against a LOCAL http server (this sandbox's
    /// proxy breaks outbound TLS for rustls clients, and search
    /// providers bot-wall datacenter clients — both verified by hand;
    /// TLS transport and search are device-verified instead).
    /// Runs only when NB_NET_LOCAL_PORT is set to the server's port.
    #[test]
    fn local_transport_when_enabled() {
        let Ok(port) = std::env::var("NB_NET_LOCAL_PORT") else {
            return;
        };
        let base = format!("http://127.0.0.1:{port}");
        let text = fetch_page_text(&format!("{base}/page.html")).unwrap();
        assert!(text.contains("天満川の水位"), "{text}");
        assert!(!text.contains("script"), "{text}");
        let dest = std::env::temp_dir().join("nb-net-local-dl.bin");
        let n = download_to_file(&format!("{base}/blob.bin"), &dest, |_, _| {}).unwrap();
        assert_eq!(n, 10_000);
        assert_eq!(std::fs::metadata(&dest).unwrap().len(), 10_000);
        let _ = std::fs::remove_file(dest);
    }
}
