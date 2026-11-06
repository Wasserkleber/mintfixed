//! Opt-in, in-memory timing diagnostics for the mod.io provider.
//! No credentials, request headers, signed URLs, or query strings are recorded.
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant, SystemTime};

mod metrics;
mod timestamps;
pub use metrics::{CacheUpdateSpan, RequestSpan, TransferSpan};

const MAX_EVENTS: usize = 1000;
static ENABLED: AtomicBool = AtomicBool::new(false);
static DATA: OnceLock<Mutex<Data>> = OnceLock::new();

#[derive(Default)]
struct Data {
    started: Option<Instant>,
    started_utc: Option<SystemTime>,
    stopped_utc: Option<SystemTime>,
    events: VecDeque<timestamps::StampedEvent>,
    dropped: usize,
}

pub enum Event {
    Request {
        endpoint: String,
        elapsed_ms: u64,
        status: Option<u16>,
        failed: bool,
    },
    Retry {
        phase: &'static str,
        delay_ms: u64,
    },
    CacheHit {
        mod_id: u32,
        file_id: u32,
    },
    Download {
        mod_id: u32,
        file_id: u32,
        size: u64,
        bytes_received: u64,
        first_chunk_ms: Option<u64>,
        transfer_ms: u64,
        blob_write_ms: u64,
        total_ms: u64,
        retries: u32,
    },
}

fn data() -> &'static Mutex<Data> {
    DATA.get_or_init(|| Mutex::new(Data::default()))
}

pub fn is_running() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Start a fresh experiment. Old events are discarded so runs are comparable.
pub fn start() {
    ENABLED.store(false, Ordering::Relaxed);
    let mut data = data().lock().unwrap();
    *data = Data {
        started: Some(Instant::now()),
        started_utc: Some(SystemTime::now()),
        ..Data::default()
    };
    metrics::reset();
    ENABLED.store(true, Ordering::Relaxed);
}

pub fn stop() {
    ENABLED.store(false, Ordering::Relaxed);
    let mut data = data().lock().unwrap();
    if data.started.is_some() && data.stopped_utc.is_none() {
        data.stopped_utc = Some(SystemTime::now());
    }
}

/// Clear collected events while retaining the current capture state.
pub fn clear() {
    let mut data = data().lock().unwrap();
    data.events.clear();
    data.dropped = 0;
    data.started = Some(Instant::now());
    data.started_utc = Some(SystemTime::now());
    data.stopped_utc = None;
    metrics::reset();
}

pub fn record(event: Event) {
    if !is_running() {
        return;
    }
    let mut data = data().lock().unwrap();
    if !is_running() {
        return;
    }
    if data.events.len() == MAX_EVENTS {
        data.events.pop_front();
        data.dropped += 1;
    }
    let entry = timestamps::StampedEvent::new(event, data.started);
    metrics::record(&entry.event);
    data.events.push_back(entry);
}

pub fn elapsed_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

/// Never capture the query string or the complete CDN URL (often signed).
pub fn request_label(url: &reqwest::Url) -> String {
    if url.host_str() == Some("api.mod.io") {
        let path = url.path();
        format!("API {}", path.chars().take(160).collect::<String>())
    } else {
        "CDN/file request".to_owned()
    }
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

pub fn report() -> String {
    let data = data().lock().unwrap();
    let mut out = String::from("mintfixed mod.io download diagnostics (local, opt-in)\n");
    let _ = writeln!(
        out,
        "Capture: {}",
        if is_running() { "running" } else { "stopped" }
    );
    let _ = writeln!(
        out,
        "Time since reset: {:.1}s",
        data.started
            .map_or(0.0, |start| start.elapsed().as_secs_f64())
    );
    if let Some(time) = data.started_utc {
        let _ = writeln!(out, "Capture started (UTC): {}", timestamps::format_utc(time));
    }
    if let Some(time) = data.stopped_utc {
        let _ = writeln!(out, "Capture stopped (UTC): {}", timestamps::format_utc(time));
    }
    let mut requests = 0_usize;
    let mut request_errors = 0_usize;
    let mut retry_waits = 0_usize;
    let mut retry_ms = 0_u64;
    let mut cache_hits = 0_usize;
    let mut downloads = 0_usize;
    let mut downloaded_bytes = 0_u64;
    let mut transfer_ms = 0_u64;
    let mut wasted_bytes = 0_u64;
    for entry in &data.events {
        let event = &entry.event;
        match event {
            Event::Request { status, failed, .. } => {
                requests += 1;
                if *failed || status.is_some_and(|code| code >= 400) {
                    request_errors += 1;
                }
            }
            Event::Retry { delay_ms, .. } => {
                retry_waits += 1;
                retry_ms = retry_ms.saturating_add(*delay_ms);
            }
            Event::CacheHit { .. } => cache_hits += 1,
            Event::Download {
                size,
                bytes_received,
                transfer_ms: ms,
                ..
            } => {
                downloads += 1;
                downloaded_bytes = downloaded_bytes.saturating_add(*size);
                transfer_ms = transfer_ms.saturating_add(*ms);
                wasted_bytes = wasted_bytes.saturating_add(bytes_received.saturating_sub(*size));
            }
        }
    }
    let _ = writeln!(
        out,
        "Requests/failed responses: {requests}/{request_errors}"
    );
    let _ = writeln!(
        out,
        "Retries/backoff: {retry_waits} / {:.1}s",
        retry_ms as f64 / 1000.0
    );
    let _ = writeln!(out, "Blob cache hits: {cache_hits}");
    let _ = writeln!(
        out,
        "Completed downloads: {downloads} ({:.2} MiB)",
        mib(downloaded_bytes)
    );
    let _ = writeln!(out, "Repeated transfer data: {:.2} MiB", mib(wasted_bytes));
    if transfer_ms > 0 {
        let _ = writeln!(
            out,
            "Aggregate effective rate: {:.2} MiB/s (sum of file durations; not wall-clock concurrency)",
            mib(downloaded_bytes) * 1000.0 / transfer_ms as f64
        );
    }
    let _ = writeln!(
        out,
        "Events retained: {} (oldest discarded: {})",
        data.events.len(),
        data.dropped
    );
    out.push_str(&metrics::report());
    out.push_str("\nTimeline (UTC event time and monotonic offset; order of completion):\n");
    for entry in &data.events {
        let event = &entry.event;
        let _ = write!(out, "  [{} | +{:.3}s] ",
            timestamps::format_utc(entry.recorded_at), entry.since_start.as_secs_f64());
        match event {
            Event::Request {
                endpoint,
                elapsed_ms,
                status,
                failed,
            } => {
                let _ = writeln!(
                    out,
                    "request {endpoint}: {elapsed_ms} ms, status {}, transport error {failed}",
                    status.map_or_else(|| "none".to_owned(), |s| s.to_string())
                );
            }
            Event::Retry { phase, delay_ms } => {
                let expected = entry.recorded_at
                    .checked_add(Duration::from_millis(*delay_ms))
                    .map(timestamps::format_utc)
                    .unwrap_or_else(|| "unknown".to_owned());
                let _ = writeln!(out,
                    "retry {phase}: scheduled backoff {delay_ms} ms, expected wake {expected} (around +{:.3}s)",
                    entry.since_start.as_secs_f64() + *delay_ms as f64 / 1000.0);
            }
            Event::CacheHit { mod_id, file_id } => {
                let _ = writeln!(out, "cache hit: mod {mod_id}, file {file_id}");
            }
            Event::Download {
                mod_id,
                file_id,
                size,
                bytes_received,
                first_chunk_ms,
                transfer_ms,
                blob_write_ms,
                total_ms,
                retries,
            } => {
                let rate = if *transfer_ms > 0 {
                    mib(*size) * 1000.0 / *transfer_ms as f64
                } else {
                    0.0
                };
                let first_chunk =
                    first_chunk_ms.map_or_else(|| "unknown".to_owned(), |ms| format!("{ms} ms"));
                let _ = writeln!(
                    out,
                    "mod {mod_id}, file {file_id}: {:.2} MiB, first chunk {}, download loop {} ms ({:.2} MiB/s), blob write {} ms, total {} ms, retries {}, transferred {:.2} MiB",
                    mib(*size),
                    first_chunk,
                    transfer_ms,
                    rate,
                    blob_write_ms,
                    total_ms,
                    retries,
                    mib(*bytes_received)
                );
            }
        }
    }
    out.push_str("\nNotes: timestamps mark when events were recorded (requests at response headers), not request start.\n");
    out.push_str("UTC dates and +seconds offsets let you compare requests, retries, and downloads.\n");
    out.push_str("Retry wake times are scheduled deadlines; the request can resume later.\n");
    out.push_str("\nNotes: transfer time includes stream processing, progress-channel waits, retries and backoff.\n");
    out.push_str(
        "An API request duration ends at response headers, not after reading the response body.\n",
    );
    out.push_str(
        "Cached downloads do not measure network speed; use fresh files for a cold-cache test.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_download_urls_are_not_logged() {
        let url =
            reqwest::Url::parse("https://cdn.example.com/secret/file.zip?token=SENSITIVE").unwrap();
        assert_eq!(request_label(&url), "CDN/file request");
        let api =
            reqwest::Url::parse("https://api.mod.io/v1/games/2475/mods?access_token=SENSITIVE")
                .unwrap();
        assert_eq!(request_label(&api), "API /v1/games/2475/mods");
    }
}
