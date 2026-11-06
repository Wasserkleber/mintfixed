//! Union of scheduled retry/backoff windows, measured on the monotonic capture clock.
//! A scheduled backoff is not proof that the task actually slept until its deadline.
use std::fmt::Write as _;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[derive(Default)]
struct Stats {
    count: u64,
    sum_ms: u128,
    // Sorted, disjoint half-open ranges [start_ms, end_ms).
    windows: Vec<(u128, u128)>,
}

static STATE: OnceLock<Mutex<Stats>> = OnceLock::new();
fn state() -> &'static Mutex<Stats> {
    STATE.get_or_init(|| Mutex::new(Stats::default()))
}

pub(super) fn reset() { *state().lock().unwrap() = Stats::default(); }

fn add_interval(s: &mut Stats, start_ms: u128, delay_ms: u64) {
    if delay_ms == 0 { return; }
    let end_ms = start_ms.saturating_add(u128::from(delay_ms));
    s.count += 1;
    s.sum_ms = s.sum_ms.saturating_add(u128::from(delay_ms));
    // Merge overlapping or touching scheduled windows; no double counting.
    let mut start = start_ms;
    let mut end = end_ms;
    let mut pos = 0;
    while pos < s.windows.len() && s.windows[pos].1 < start { pos += 1; }
    let first = pos;
    while pos < s.windows.len() && s.windows[pos].0 <= end {
        start = start.min(s.windows[pos].0);
        end = end.max(s.windows[pos].1);
        pos += 1;
    }
    s.windows.splice(first..pos, [(start, end)]);
}

pub(super) fn record(since_start: Duration, delay_ms: u64) {
    let mut s = state().lock().unwrap();
    add_interval(&mut s, since_start.as_millis(), delay_ms);
}

pub(super) fn report() -> String {
    let s = state().lock().unwrap();
    let union_ms: u128 = s.windows.iter().map(|(a, b)| b - a).sum();
    let overlap_ms = s.sum_ms.saturating_sub(union_ms);
    let mut out = String::from("\nRetry-window overlap (uncapped):\n");
    let _ = writeln!(out, "  Scheduled waits: {} ({:.3}s summed)", s.count, s.sum_ms as f64 / 1000.0);
    let _ = writeln!(out, "  Distinct scheduled waiting time (union): {:.3}s across {} windows", union_ms as f64 / 1000.0, s.windows.len());
    let _ = writeln!(out, "  Overlapping scheduled time removed: {:.3}s", overlap_ms as f64 / 1000.0);
    out.push_str("  This is the union of planned retry windows, NOT measured task sleep or total operation wall time.\n");
    for (start, end) in s.windows.iter().take(12) {
        let _ = writeln!(out, "    +{:.3}s to +{:.3}s (planned)", *start as f64 / 1000.0, *end as f64 / 1000.0);
    }
    if s.windows.len() > 12 { out.push_str("    (additional windows omitted from display; included in totals)\n"); }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlapping_waits_have_a_distinct_union() {
        let mut s = Stats::default();
        add_interval(&mut s, 1000, 60000);
        add_interval(&mut s, 2000, 59000);
        add_interval(&mut s, 71000, 2000);
        assert_eq!(s.count, 3);
        assert_eq!(s.sum_ms, 121000);
        assert_eq!(s.windows, vec![(1000, 61000), (71000, 73000)]);
        let union: u128 = s.windows.iter().map(|(a,b)| b-a).sum();
        assert_eq!(union, 62000);
    }
    #[test]
    fn touching_intervals_merge_and_zero_is_ignored() {
        let mut s = Stats::default();
        add_interval(&mut s, 10, 10);
        add_interval(&mut s, 20, 10);
        add_interval(&mut s, 40, 0);
        assert_eq!(s.windows, vec![(10, 30)]);
        assert_eq!(s.count, 2);
    }
}
