//! Extended opt-in download metrics. This module never stores URLs, tokens or headers.
use super::Event;
use std::fmt::Write as _;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const LABELS: [&str; 6] = [
    "Mod listing/search", "Mod details", "Mod files", "Dependencies",
    "Other mod.io API", "CDN requests",
];

#[derive(Clone, Copy, Default)]
struct RequestTotals {
    count: u64,
    errors: u64,
    rate_limited: u64,
    duration_ms: u64,
}

#[derive(Default)]
struct Metrics {
    generation: u64,
    by_operation: [RequestTotals; 6],
    requests_during_cache_update: u64,
    active_api: u64,
    peak_api: u64,
    active_cdn: u64,
    peak_cdn: u64,
    active_transfers: u64,
    peak_transfers: u64,
    started_transfers: u64,
    finished_transfers: u64,
    transfer_first_start: Option<Instant>,
    transfer_last_end: Option<Instant>,
    transfer_wave_start: Option<Instant>,
    transfer_active_ms: u64,
    downloaded_bytes: u64,
    active_cache_updates: u64,
    peak_cache_updates: u64,
    cache_updates_started: u64,
    cache_updates_finished: u64,
    cache_total_ms: u64,
    cache_last_ms: Option<u64>,
    cache_longest_ms: u64,
}

static METRICS: OnceLock<Mutex<Metrics>> = OnceLock::new();

fn state() -> &'static Mutex<Metrics> {
    METRICS.get_or_init(|| Mutex::new(Metrics::default()))
}

pub(super) fn reset() {
    let mut m = state().lock().unwrap();
    let generation = m.generation.wrapping_add(1);
    *m = Metrics { generation, ..Metrics::default() };
}

// Input is already sanitized by request_label(); no query strings are inspected.
fn operation(endpoint: &str) -> usize {
    if !endpoint.starts_with("API ") {
        return 5;
    }
    let path = endpoint.trim_start_matches("API ");
    if path.ends_with("/dependencies") { 3 }
    else if path.ends_with("/files") || path.contains("/files/") { 2 }
    else if path.ends_with("/mods") { 0 }
    else if path.contains("/mods/") { 1 }
    else { 4 }
}

pub(super) fn record(event: &Event) {
    let mut m = state().lock().unwrap();
    match event {
        Event::Request { endpoint, elapsed_ms, status, failed } => {
            let index = operation(endpoint);
            let group = &mut m.by_operation[index];
            group.count += 1;
            if *failed || status.is_some_and(|code| code >= 400) {
                group.errors += 1;
            }
            if *status == Some(429) {
                group.rate_limited += 1;
            }
            group.duration_ms = group.duration_ms.saturating_add(*elapsed_ms);
            if index != 5 && m.active_cache_updates > 0 {
                // Overlapping operations can also issue API requests here.
                m.requests_during_cache_update += 1;
            }
        }
        Event::Download { size, .. } => {
            m.downloaded_bytes = m.downloaded_bytes.saturating_add(*size);
        }
        _ => {}
    }
}

/// Counts requests only while the actual HTTP request is in flight, excluding backoff.
pub struct RequestSpan {
    generation: u64,
    is_api: bool,
}

impl RequestSpan {
    pub fn begin(url: &reqwest::Url) -> Option<Self> {
        if !super::is_running() { return None; }
        let is_api = url.host_str() == Some("api.mod.io");
        let mut m = state().lock().unwrap();
        if !super::is_running() { return None; }
        if is_api {
            m.active_api += 1;
            m.peak_api = m.peak_api.max(m.active_api);
        } else {
            m.active_cdn += 1;
            m.peak_cdn = m.peak_cdn.max(m.active_cdn);
        }
        Some(Self { generation: m.generation, is_api })
    }
}

impl Drop for RequestSpan {
    fn drop(&mut self) {
        let mut m = state().lock().unwrap();
        if m.generation != self.generation { return; }
        if self.is_api { m.active_api = m.active_api.saturating_sub(1); }
        else { m.active_cdn = m.active_cdn.saturating_sub(1); }
    }
}

/// Time the actual download loop, including stream retries but excluding disk writes.
pub struct TransferSpan { generation: u64 }

impl TransferSpan {
    pub fn begin() -> Option<Self> {
        if !super::is_running() { return None; }
        let mut m = state().lock().unwrap();
        if !super::is_running() { return None; }
        let now = Instant::now();
        if m.active_transfers == 0 { m.transfer_wave_start = Some(now); }
        if m.transfer_first_start.is_none() { m.transfer_first_start = Some(now); }
        m.started_transfers += 1;
        m.active_transfers += 1;
        m.peak_transfers = m.peak_transfers.max(m.active_transfers);
        Some(Self { generation: m.generation })
    }
}

impl Drop for TransferSpan {
    fn drop(&mut self) {
        let mut m = state().lock().unwrap();
        if self.generation != m.generation { return; }
        if m.active_transfers == 0 { return; }
        let now = Instant::now();
        m.active_transfers -= 1;
        m.finished_transfers += 1;
        m.transfer_last_end = Some(now);
        if m.active_transfers == 0 {
            if let Some(start) = m.transfer_wave_start.take() {
                m.transfer_active_ms = m.transfer_active_ms.saturating_add(super::elapsed_ms(now.duration_since(start)));
            }
        }
    }
}

/// Time the full mod-store cache-update function including all providers and waits.
pub struct CacheUpdateSpan {
    generation: u64,
    started: Instant,
}

impl CacheUpdateSpan {
    pub fn begin() -> Option<Self> {
        if !super::is_running() { return None; }
        let mut m = state().lock().unwrap();
        if !super::is_running() { return None; }
        m.cache_updates_started += 1;
        m.active_cache_updates += 1;
        m.peak_cache_updates = m.peak_cache_updates.max(m.active_cache_updates);
        Some(Self { generation: m.generation, started: Instant::now() })
    }
}

impl Drop for CacheUpdateSpan {
    fn drop(&mut self) {
        let duration = super::elapsed_ms(self.started.elapsed());
        let mut m = state().lock().unwrap();
        if self.generation != m.generation { return; }
        m.cache_updates_finished += 1;
        m.active_cache_updates = m.active_cache_updates.saturating_sub(1);
        m.cache_last_ms = Some(duration);
        m.cache_total_ms = m.cache_total_ms.saturating_add(duration);
        m.cache_longest_ms = m.cache_longest_ms.max(duration);
    }
}

fn sec(ms: u64) -> f64 { ms as f64 / 1000.0 }

pub(super) fn report() -> String {
    let m = state().lock().unwrap();
    let mut out = String::from("\nExtended benchmark summary (uncapped counters):\n");
    let _ = writeln!(out, "Requests by operation (HTTP 429 shown separately):");
    for (label, stat) in LABELS.iter().zip(m.by_operation.iter()) {
        if stat.count > 0 {
            let _ = writeln!(out,
                "  {label}: {} requests, {} errors ({} HTTP 429), avg headers {:.0} ms",
                stat.count, stat.errors, stat.rate_limited,
                stat.duration_ms as f64 / stat.count as f64);
        }
    }
    let _ = writeln!(out,
        "  API requests while cache-update scope was active: {} (approximate: other work may overlap)",
        m.requests_during_cache_update);
    let _ = writeln!(out,
        "Peak in-flight HTTP requests: API {}, CDN {} (currently API {}, CDN {})",
        m.peak_api, m.peak_cdn, m.active_api, m.active_cdn);
    let _ = writeln!(out,
        "Peak simultaneous download loops: {} (started {}, finished {}, currently {})",
        m.peak_transfers, m.started_transfers, m.finished_transfers, m.active_transfers);
    let elapsed_wall_ms = match (m.transfer_first_start, m.transfer_last_end) {
        (Some(start), Some(end)) if end >= start => {
            let endpoint = if m.active_transfers > 0 { Instant::now() } else { end };
            Some(super::elapsed_ms(endpoint.duration_since(start)))
        }
        (Some(start), None) if m.active_transfers > 0 => {
            Some(super::elapsed_ms(start.elapsed()))
        }
        _ => None,
    };
    let active_ms = m.transfer_active_ms.saturating_add(
        m.transfer_wave_start.map_or(0, |start| super::elapsed_ms(start.elapsed()))
    );
    if let Some(ms) = elapsed_wall_ms {
        let _ = writeln!(out, "Download wall-clock span: {:.3}s (first loop start through last loop end; includes gaps)", sec(ms));
    } else {
        out.push_str("Download wall-clock span: no measured transfers\n");
    }
    let _ = writeln!(out, "Time with >=1 download active: {:.3}s (union of overlapping loops)", sec(active_ms));
    if active_ms > 0 {
        let _ = writeln!(out,
            "Wall-clock active transfer throughput: {:.2} MiB/s (successful bytes / active time)",
            m.downloaded_bytes as f64 / 1_048_576.0 / sec(active_ms));
    }
    let _ = writeln!(out,
        "Cache update operations: started {}, finished {}, active {}, peak concurrent {}",
        m.cache_updates_started, m.cache_updates_finished, m.active_cache_updates,
        m.peak_cache_updates);
    if let Some(last_ms) = m.cache_last_ms {
        let _ = writeln!(out,
            "Cache update time: last {:.3}s, longest {:.3}s, sum {:.3}s (sum may overlap)",
            sec(last_ms), sec(m.cache_longest_ms), sec(m.cache_total_ms));
    }
    out.push_str("Counters include all completed requests in the capture, even if the timeline dropped old entries.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_operations_do_not_need_query_strings_or_tokens() {
        assert_eq!(operation("API /v1/games/2475/mods"), 0);
        assert_eq!(operation("API /v1/games/2475/mods/100/files"), 2);
        assert_eq!(operation("API /v1/games/2475/mods/100/dependencies"), 3);
        assert_eq!(operation("API /v1/games/2475/mods/100"), 1);
        assert_eq!(operation("CDN/file request"), 5);
    }

    #[test]
    fn spans_count_peaks_and_completed_cache_updates() {
        super::super::start();
        let url = reqwest::Url::parse("https://api.mod.io/v1/games/2475/mods").unwrap();
        let first = RequestSpan::begin(&url);
        let second = RequestSpan::begin(&url);
        let first_transfer = TransferSpan::begin();
        let second_transfer = TransferSpan::begin();
        let cache = CacheUpdateSpan::begin();
        super::super::record(Event::Request {
            endpoint: "API /v1/games/2475/mods".to_owned(),
            elapsed_ms: 100, status: Some(429), failed: false,
        });
        drop((first, second, first_transfer, second_transfer, cache));
        super::super::stop();
        let summary = report();
        assert!(summary.contains("Mod listing/search: 1 requests, 1 errors (1 HTTP 429)"));
        assert!(summary.contains("Peak in-flight HTTP requests: API 2, CDN 0"));
        assert!(summary.contains("Peak simultaneous download loops: 2"));
        assert!(summary.contains("Cache update operations: started 1, finished 1"));
    }
}
