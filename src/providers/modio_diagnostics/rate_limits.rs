//! Diagnostics-only HTTP 429 details. Never save request headers, query strings,
//! response text, or credentials. Retry-After is the *only* response header kept.
use std::fmt::Write as _;
use std::sync::{atomic::{AtomicU64, Ordering}, Mutex, OnceLock};
use std::time::Duration;

const MAX_429_RECORDS: usize = 2000;
const MAX_BODY_BYTES: usize = 8192;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static STATE: OnceLock<Mutex<State>> = OnceLock::new();

#[derive(Default)]
struct State {
    records: Vec<RateLimitRecord>,
    dropped: usize,
}

struct RateLimitRecord {
    id: u64,
    when: Duration,
    endpoint: String,
    origin: &'static str,
    retry_after_raw: &'static str,
    retry_after_text: Option<String>,
    error_ref: Option<u32>,
    inspection: &'static str,
}

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| Mutex::new(State::default()))
}

pub(super) fn reset() {
    *state().lock().unwrap() = State::default();
}

/// Allow the literal Retry-After header but no other header values.
/// Invalid or excessive values are marked, never dumped into reports.
fn safe_retry_after(raw: &reqwest::header::HeaderValue) -> (&'static str, Option<String>) {
    let Ok(s) = raw.to_str() else { return ("invalid/non-ASCII", None); };
    if s.len() > 128 || !s.bytes().all(|c| c == b' ' || (0x21..=0x7e).contains(&c)) {
        return ("invalid/overlong", None);
    }
    ("present", Some(s.to_owned()))
}

/// Called for every HTTP 429, including terminal responses forwarded to modio-rs.
/// Does not inspect (or consume) the HTTP body.
pub(super) fn observe(request: &reqwest::Request, response: &reqwest::Response) -> Option<u64> {
    if !super::is_running() || response.status().as_u16() != 429 ||
        request.url().host_str() != Some("api.mod.io") {
        return None;
    }
    let (retry_after_raw, retry_after_text) = response.headers()
        .get(reqwest::header::RETRY_AFTER)
        .map(safe_retry_after)
        .unwrap_or(("missing", None));
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let when = super::elapsed_since_capture();
    let mut s = state().lock().unwrap();
    if s.records.len() >= MAX_429_RECORDS {
        s.records.remove(0);
        s.dropped += 1;
    }
    s.records.push(RateLimitRecord {
        id,
        when,
        endpoint: super::request_label(request.url()),
        origin: super::current_origin(),
        retry_after_raw,
        retry_after_text,
        error_ref: None,
        inspection: "not inspected (response may be forwarded)",
    });
    Some(id)
}

fn parse_error_ref(bytes: &[u8]) -> Option<u32> {
    let json: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let error_ref = json.get("error")?.get("error_ref")?.as_u64()?;
    u32::try_from(error_ref).ok()
}

fn update(id: u64, body: &[u8], inspection: &'static str) {
    let mut s = state().lock().unwrap();
    if let Some(record) = s.records.iter_mut().find(|record| record.id == id) {
        record.error_ref = parse_error_ref(body);
        record.inspection = inspection;
    }
}

/// Called ONLY after Mintfixed has decided to retry, where it discards the
/// current response anyway. Never called for a response returned to the SDK.
/// Body inspection is limited to 8 KiB and never stored in diagnostics.
pub(super) async fn inspect_discarded(id: u64, response: &mut reqwest::Response) {
    if !super::is_running() || response.status().as_u16() != 429 { return; }
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if chunk.len() > MAX_BODY_BYTES - body.len() {
                    update(id, &[], "body exceeded 8 KiB; not parsed");
                    return;
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => {
                update(id, &body, "discarded retry body inspected");
                return;
            }
            Err(_) => {
                update(id, &[], "body read failed");
                return;
            }
        }
    }
}

pub(super) fn report() -> String {
    let s = state().lock().unwrap();
    let mut out = String::from("\nHTTP 429 response details (all observed responses):\n");
    if s.records.is_empty() { out.push_str("  No HTTP 429 responses recorded.\n"); }
    for record in &s.records {
        let meaning = match record.error_ref {
            Some(11008) => "global limit",
            Some(11009) => "endpoint limit",
            Some(_) => "other error ref",
            None => "not available",
        };
        let code = record.error_ref.map_or_else(|| "unknown".to_owned(), |x| x.to_string());
        let retry_after = match &record.retry_after_text {
            Some(text) => format!("{text:?} (raw)"),
            None => record.retry_after_raw.to_owned(),
        };
        let _ = writeln!(out,
            "  +{:.3}s {} origin {}: error_ref {} ({}), Retry-After {}, body {}",
            record.when.as_secs_f64(), record.endpoint, record.origin,
            code, meaning, retry_after, record.inspection);
    }
    if s.dropped != 0 {
        let _ = writeln!(out, "  Older 429 details discarded: {} (capture limit {})", s.dropped, MAX_429_RECORDS);
    }
    out.push_str("  error_ref is from the mod.io JSON body only when Mintfixed discards that body before a retry.\n");
    out.push_str("  Terminal/forwarded 429 bodies are not consumed; their error_ref may be unknown.\n");
    out.push_str("  Raw Retry-After is never modified into the fallback/backoff duration; compare with Retry events below.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_global_and_endpoint_error_refs() {
        assert_eq!(parse_error_ref(br#"{"error":{"code":429,"error_ref":11008,"message":"secret"}}"#), Some(11008));
        assert_eq!(parse_error_ref(br#"{"error":{"code":429,"error_ref":11009}}"#), Some(11009));
        assert_eq!(parse_error_ref(b"not json"), None);
        assert_eq!(parse_error_ref(br#"{"error":{"message":"unknown"}}"#), None);
    }
    #[test]
    fn header_is_preserved_without_leaking_unsafe_values() {
        let raw = reqwest::header::HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT");
        let (state, text) = safe_retry_after(&raw);
        assert_eq!(state, "present");
        assert_eq!(text.as_deref(), Some("Wed, 21 Oct 2015 07:28:00 GMT"));
        let raw = reqwest::header::HeaderValue::from_static("0");
        assert_eq!(safe_retry_after(&raw).1.as_deref(), Some("0"));
        let raw = reqwest::header::HeaderValue::from_str(&"1".repeat(129)).unwrap();
        assert_eq!(safe_retry_after(&raw).1, None);
    }
}
