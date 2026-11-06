//! Opt-in request origin and privacy-preserving request fingerprint diagnostics.
//! Sensitive query values are NEVER stored. The fingerprint includes public numeric
//! selectors and known sort keys only; it is not a full request-content hash.
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};

tokio::task_local! {
    static REQUEST_ORIGIN: &'static str;
}

/// Set the top-level operation for this async task. Nested provider calls keep
/// their existing parent operation, so a cache refresh stays attributed to the
/// cache refresh instead of becoming a generic mod.io helper call.
pub async fn with_origin<F: Future>(origin: &'static str, future: F) -> F::Output {
    let _operation_span = OperationSpan::begin(origin);
    if REQUEST_ORIGIN.try_with(|_| ()).is_ok() {
        future.await
    } else {
        REQUEST_ORIGIN.scope(origin, future).await
    }
}


/// Track the full lifetime of an async operation, including awaited retries.
/// The guard also records unwinding/cancellation as an exit (not a success).
struct OperationSpan {
    origin: &'static str,
    started: std::time::Instant,
    generation: u64,
}

#[derive(Default)]
struct OperationTiming {
    started: u64,
    exited: u64,
    active: u64,
    peak_active: u64,
    total_ns: u128,
    max_ns: u128,
    last_ns: u128,
}

impl OperationSpan {
    fn begin(origin: &'static str) -> Option<Self> {
        if !super::is_running() { return None; }
        let mut m = state().lock().unwrap();
        if !super::is_running() { return None; }
        let generation = m.generation;
        let stat = m.operation_timing.entry(origin).or_default();
        stat.started += 1;
        stat.active += 1;
        stat.peak_active = stat.peak_active.max(stat.active);
        Some(Self { origin, started: std::time::Instant::now(), generation })
    }
}

impl Drop for OperationSpan {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed().as_nanos();
        let mut m = state().lock().unwrap();
        if m.generation != self.generation { return; }
        let stat = m.operation_timing.entry(self.origin).or_default();
        stat.exited += 1;
        stat.active = stat.active.saturating_sub(1);
        stat.total_ns = stat.total_ns.saturating_add(elapsed);
        stat.max_ns = stat.max_ns.max(elapsed);
        stat.last_ns = elapsed;
    }
}

/// Secret-free parameter shape. The contents of unknown keys are discarded.
/// Numeric public selectors are reduced to value types to make repeated
/// per-mod requests comparable without emitting full query strings.
fn safe_selector_shape(request: &reqwest::Request) -> String {
    let mut selectors = Vec::<String>::new();
    let mut omitted = 0_usize;
    for (key, value) in request.url().query_pairs() {
        let key: &str = key.as_ref();
        let value: &str = value.as_ref();
        if is_public_numeric_key(key) {
            let kind = if value.len() <= 80 && value.bytes().all(|c| c.is_ascii_digit()) {
                if matches!(key, "_limit" | "limit" | "_offset" | "offset") {
                    format!("{key}={value}")
                } else {
                    format!("{key}=<number>")
                }
            } else if value.len() <= 80 && value.bytes().all(|c| c.is_ascii_digit() || c == b',') {
                format!("{key}=<numeric-list>")
            } else {
                format!("{key}=<redacted>")
            };
            selectors.push(kind);
        } else if key == "_sort" || key == "sort" {
            let kind = match value {
                "date_updated" | "date_added" | "date_live" | "downloads" |
                "popularity" | "rating" | "name" | "id" | "-date_updated" |
                "-date_added" | "-downloads" | "-popularity" => value,
                _ => "<other>",
            };
            selectors.push(format!("{key}={kind}"));
        } else {
            omitted += 1;
        }
    }
    selectors.sort();
    if omitted != 0 { selectors.push(format!("{omitted} other key(s) omitted")); }
    if selectors.is_empty() { "(none)".to_owned() } else { selectors.join(", ") }
}

/// Only known API request shapes are recorded. Unknown query keys, their
/// values, CDN locations, and authentication headers are never retained.
pub fn record_request_profile(request: &reqwest::Request) {
    if !super::is_running() || request.url().host_str() != Some("api.mod.io") { return; }
    let origin = current_origin();
    let endpoint = super::request_label(request.url());
    let category = if endpoint.ends_with("/files") || endpoint.contains("/files/") {
        "Mod files"
    } else if endpoint.ends_with("/dependencies") {
        "Dependencies"
    } else if endpoint.ends_with("/mods") { "Mod listing/search" }
    else if endpoint.contains("/mods/") { "Mod details" }
    else { "Other API" };
    let shape = format!("{} {}", request.method(), safe_selector_shape(request));
    let mut m = state().lock().unwrap();
    *m.request_profiles.entry((category, origin, shape.clone())).or_default() += 1;
    // Retain only a small, capped set of concrete API path examples; these
    // are already present in the v4 timeline and contain no query string.
    if category == "Mod files" {
        let key = (endpoint, origin, shape);
        if m.request_examples.contains_key(&key) || m.request_examples.len() < 600 {
            *m.request_examples.entry(key).or_default() += 1;
        }
    }
}

pub fn current_origin() -> &'static str {
    REQUEST_ORIGIN.try_with(|origin| *origin).unwrap_or("unattributed")
}

fn is_public_numeric_key(key: &str) -> bool {
    matches!(key,
        "_offset" | "_limit" | "offset" | "limit" | "id" | "id-eq" |
        "id-in" | "mod_id" | "mod_id-eq" | "mod_id-in" |
        "file_id" | "file_id-eq" | "status" | "status-eq" |
        "date_updated-min" | "date_updated-max" | "date_live-min")
}

fn public_query_fingerprint(url: &reqwest::Url) -> String {
    // Include only explicitly allowlisted *public* selectors. Never include
    // tokens, API keys, signatures, unknown keys or arbitrary filter values.
    let mut known = Vec::new();
    for (key, value) in url.query_pairs() {
        let key: &str = key.as_ref();
        let value: &str = value.as_ref();
        if is_public_numeric_key(key) {
            if value.len() <= 80 && value.bytes().all(|c| c.is_ascii_digit() || c == b',') {
                known.push((key.to_owned(), value.to_owned()));
            } else {
                known.push((key.to_owned(), "[non-numeric]".to_owned()));
            }
        } else if key == "_sort" || key == "sort" {
            let safe_sort = matches!(value,
                "date_updated" | "date_added" | "date_live" | "downloads" |
                "popularity" | "rating" | "name" | "id" | "-date_updated" |
                "-date_added" | "-downloads" | "-popularity");
            known.push((key.to_owned(), if safe_sort { value.to_owned() } else { "[other-sort]".to_owned() }));
        }
    }
    known.sort();
    let mut canonical = format!("{}:{}", url.host_str().unwrap_or(""), url.path());
    for (key, value) in known {
        canonical.push('|');
        canonical.push_str(&key);
        canonical.push('=');
        canonical.push_str(&value);
    }
    canonical
}

pub fn fingerprint(request: &reqwest::Request) -> String {
    if request.url().host_str() != Some("api.mod.io") {
        return "CDN (redacted)".to_owned();
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    request.method().as_str().hash(&mut h);
    public_query_fingerprint(request.url()).hash(&mut h);
    format!("{:016x}", h.finish())
}

#[derive(Default)]
struct RequestCount {
    attempts: u64,
    success: u64,
    errors: u64,
    rate_limited: u64,
    origins: BTreeMap<&'static str, u64>,
}

#[derive(Default)]
struct OriginMetrics {
    // Keyed by sanitized endpoint and lossy, secret-free fingerprint.
    fingerprints: BTreeMap<(String, String), RequestCount>,
    requests_by_origin: BTreeMap<&'static str, RequestCount>,
    generation: u64,
    operation_timing: BTreeMap<&'static str, OperationTiming>,
    request_profiles: BTreeMap<(&'static str, &'static str, String), u64>,
    request_examples: BTreeMap<(String, &'static str, String), u64>,
}

static STATE: OnceLock<Mutex<OriginMetrics>> = OnceLock::new();
fn state() -> &'static Mutex<OriginMetrics> {
    STATE.get_or_init(|| Mutex::new(OriginMetrics::default()))
}

pub(super) fn reset() {
    let mut m = state().lock().unwrap();
    let generation = m.generation.wrapping_add(1);
    *m = OriginMetrics { generation, ..OriginMetrics::default() };
}

fn add(count: &mut RequestCount, origin: &'static str, status: Option<u16>, failed: bool) {
    count.attempts += 1;
    if !failed && status.is_some_and(|s| (200..400).contains(&s)) { count.success += 1; }
    if failed || status.is_some_and(|s| s >= 400) { count.errors += 1; }
    if status == Some(429) { count.rate_limited += 1; }
    *count.origins.entry(origin).or_default() += 1;
}

pub(super) fn record(
    endpoint: &str,
    origin: &'static str,
    fingerprint: &str,
    status: Option<u16>,
    failed: bool,
) {
    let mut m = state().lock().unwrap();
    add(m.requests_by_origin.entry(origin).or_default(), origin, status, failed);
    if endpoint.starts_with("API ") {
        add(m.fingerprints.entry((endpoint.to_owned(), fingerprint.to_owned())).or_default(),
            origin, status, failed);
    }
}

pub(super) fn report() -> String {
    let m = state().lock().unwrap();
    let mut out = String::from("\nRequest origins (uncapped; top-level async operation, not a call stack):\n");
    for (origin, count) in &m.requests_by_origin {
        let _ = writeln!(out, "  {origin}: {} attempts, {} successful, {} errors ({} HTTP 429)",
            count.attempts, count.success, count.errors, count.rate_limited);
    }
    out.push_str("Repeated API fingerprints (successful responses, up to 20):\n");
    let mut repeated: Vec<_> = m.fingerprints.iter()
        .filter(|(_, counts)| counts.success > 1)
        .collect();
    repeated.sort_by(|a,b| b.1.success.cmp(&a.1.success).then_with(|| a.0.cmp(b.0)));
    for ((endpoint, fingerprint), counts) in repeated.into_iter().take(20) {
        let sources = counts.origins.iter()
            .map(|(origin, n)| format!("{origin}={n}"))
            .collect::<Vec<_>>().join(", ");
        let _ = writeln!(out, "  {} x{} ({} attempts, {} HTTP 429): fp={} origins [{}]",
            endpoint, counts.success, counts.attempts, counts.rate_limited, fingerprint, sources);
    }
    out.push_str("Fingerprint uses API method/path and allowlisted public numeric/sort selectors;\n");
    out.push_str("unknown query values are excluded, so equal fingerprints are candidates, not proof of identical HTTP requests.\n");
    out.push_str("CDN URLs, query strings, access tokens, signed parameters and headers are never recorded.\n");

    out.push_str("\nAsync operation lifetime (includes awaits/backoff, not just response headers):\n");
    let mut operations: Vec<_> = m.operation_timing.iter().collect();
    operations.sort_by(|a,b| b.1.total_ns.cmp(&a.1.total_ns));
    for (name, stat) in operations.into_iter().take(30) {
        let _ = writeln!(out,
            "  {}: started {}, exited {} (including cancellation), active {}, peak {}, last {:.3}s, max {:.3}s, sum {:.3}s",
            name, stat.started, stat.exited, stat.active, stat.peak_active,
            stat.last_ns as f64 / 1e9, stat.max_ns as f64 / 1e9, stat.total_ns as f64 / 1e9);
    }
    out.push_str("  Sums overlap for concurrently running operations; no operation return values or success/failure are inferred.\n");
    out.push_str("\nSafe request selector shapes (top 25, grouped across mod IDs):\n");
    let mut profiles: Vec<_> = m.request_profiles.iter().collect();
    profiles.sort_by(|a,b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    for ((category, origin, shape), count) in profiles.into_iter().take(25) {
        let _ = writeln!(out, "  {} | {} | {}: {} requests", category, origin, shape, count);
    }
    out.push_str("  Shapes show only allowlisted public numeric/sort selectors; all other parameters are omitted.\n");
    out.push_str("\nCross-operation mod-file request examples (up to 6 mod paths):\n");
    let mut paths: BTreeMap<&str, Vec<(&str, &str, u64)>> = BTreeMap::new();
    for ((path, origin, shape), count) in &m.request_examples {
        paths.entry(path.as_str()).or_default().push((*origin, shape.as_str(), *count));
    }
    let mut printed = 0;
    for (path, entries) in paths {
        let mut unique_origins: Vec<_> = entries.iter().map(|(origin, _, _)| *origin).collect();
        unique_origins.sort_unstable();
        unique_origins.dedup();
        if unique_origins.len() < 2 { continue; }
        let _ = writeln!(out, "  {path}:");
        for (origin, shape, count) in entries.iter().take(6) {
            let _ = writeln!(out, "    {origin}: {shape} ({count} requests)");
        }
        printed += 1;
        if printed >= 6 { break; }
    }
    if printed == 0 { out.push_str("  No cross-origin mod file paths observed in this capture.\n"); }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(url: &str) -> reqwest::Request {
        reqwest::Request::new(reqwest::Method::GET, url.parse().unwrap())
    }

    #[test]
    fn private_query_parameters_never_escape_safe_selector_shape() {
        let request = req("https://api.mod.io/v1/games/2475/mods/12/files?access_token=SECRET&id-eq=123&_limit=100&custom=VALUE");
        let shape = safe_selector_shape(&request);
        assert!(shape.contains("id-eq=<number>"));
        assert!(shape.contains("_limit=100"));
        assert!(shape.contains("2 other key(s) omitted"));
        assert!(!shape.contains("SECRET"));
        assert!(!shape.contains("VALUE"));
        assert!(!shape.contains("access_token"));
        assert!(!shape.contains("custom"));
    }

    #[test]
    fn file_selectors_remain_comparable_without_specific_mod_ids() {
        let a = req("https://api.mod.io/v1/games/2475/mods/12/files?id-eq=111");
        let b = req("https://api.mod.io/v1/games/2475/mods/15/files?id-eq=222");
        let c = req("https://api.mod.io/v1/games/2475/mods/12/files?status-eq=1");
        assert_eq!(safe_selector_shape(&a), safe_selector_shape(&b));
        assert_ne!(safe_selector_shape(&a), safe_selector_shape(&c));
    }

    #[test]
    fn authentication_tokens_do_not_change_fingerprints() {
        let a = req("https://api.mod.io/v1/games/2475/mods?access_token=SECRET_A&_offset=20");
        let b = req("https://api.mod.io/v1/games/2475/mods?access_token=SECRET_B&_offset=20");
        assert_eq!(fingerprint(&a), fingerprint(&b));
        assert!(!fingerprint(&a).contains("SECRET"));
    }

    #[test]
    fn public_numeric_pagination_changes_fingerprint() {
        let a = req("https://api.mod.io/v1/games/2475/mods?_offset=20");
        let b = req("https://api.mod.io/v1/games/2475/mods?_offset=40");
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn signed_cdn_urls_are_never_fingerprinted() {
        let a = req("https://cdn.example.org/private/file.zip?sig=TOPSECRET");
        assert_eq!(fingerprint(&a), "CDN (redacted)");
    }

    #[tokio::test]
    async fn origin_scope_survives_await_and_preserves_parent() {
        assert_eq!(current_origin(), "unattributed");
        with_origin("install", async {
            tokio::task::yield_now().await;
            assert_eq!(current_origin(), "install");
            with_origin("modio::download", async {
                assert_eq!(current_origin(), "install");
            }).await;
        }).await;
        assert_eq!(current_origin(), "unattributed");
    }
}
