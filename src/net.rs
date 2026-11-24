use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;

pub const USER_AGENT: &str = "MangaReader/0.1 (desktop reader; local user)";

static AGENT: OnceLock<ureq::Agent> = OnceLock::new();

fn agent() -> &'static ureq::Agent {
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(40))
            .build()
    })
}

/// GET and parse JSON body.
pub fn get_json(url: &str) -> Result<serde_json::Value, String> {
    let resp = agent()
        .get(url)
        .call()
        .map_err(|e| format!("GET {url}: {e}"))?;
    resp.into_json::<serde_json::Value>()
        .map_err(|e| format!("Некорректный JSON от {url}: {e}"))
}

/// GET raw bytes (images, archives).
pub fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let resp = agent()
        .get(url)
        .call()
        .map_err(|e| format!("GET {url}: {e}"))?;
    let mut buf = Vec::new();
    resp.into_reader()
        .read_to_end(&mut buf)
        .map_err(|e| format!("Ошибка чтения {url}: {e}"))?;
    Ok(buf)
}

/// Percent-encode a query parameter value.
pub fn encode_param(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// GET bytes with a few retries (MangaDex page servers time out sometimes).
pub fn get_bytes_retry(url: &str, attempts: usize) -> Result<Vec<u8>, String> {
    let mut last = String::new();
    for i in 0..attempts {
        match get_bytes(url) {
            Ok(b) => return Ok(b),
            Err(e) => {
                last = e;
                if i + 1 < attempts {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
        }
    }
    Err(last)
}
