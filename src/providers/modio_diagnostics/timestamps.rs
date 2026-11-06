//! Human-readable UTC timestamps for opt-in mod.io diagnostics.
//! Kept separate from download code; uses only the standard library.
use super::Event;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(super) struct StampedEvent {
    pub(super) event: Event,
    /// Wall-clock time when the event was recorded (UTC).
    pub(super) recorded_at: SystemTime,
    /// Monotonic time relative to the capture start, unaffected by clock changes.
    pub(super) since_start: Duration,
}

impl StampedEvent {
    pub(super) fn new(event: Event, capture_start: Option<Instant>) -> Self {
        let now = Instant::now();
        Self {
            event,
            recorded_at: SystemTime::now(),
            since_start: capture_start.map_or(Duration::ZERO, |start| {
                now.saturating_duration_since(start)
            }),
        }
    }
}

// Gregorian civil date from days since 1970-01-01, using integer arithmetic.
// This avoids adding a new datetime dependency solely for diagnostics.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let day_of_year = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

/// ISO 8601 in UTC, including milliseconds, for correlation with Mintfixed logs.
pub(super) fn format_utc(time: SystemTime) -> String {
    // Current/future dates are normal; clamp pre-epoch or implausibly far-future
    // system clocks rather than risking an integer overflow.
    let millis = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .min(i64::MAX as u128) as i64;
    let seconds = millis.div_euclid(1_000);
    let submillis = millis.rem_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{submillis:03}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_epoch_timestamp() {
        assert_eq!(format_utc(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn utc_leap_day_and_milliseconds() {
        let time = UNIX_EPOCH + Duration::from_millis(1_709_251_199_999);
        assert_eq!(format_utc(time), "2024-02-29T23:59:59.999Z");
    }

    #[test]
    fn utc_modern_date() {
        let time = UNIX_EPOCH + Duration::from_millis(1_793_966_118_321);
        assert_eq!(format_utc(time), "2026-11-06T11:55:18.321Z");
    }

    #[test]
    fn event_offset_is_relative_to_capture_start() {
        let capture_start = Instant::now();
        let entry = StampedEvent::new(Event::CacheHit { mod_id: 1, file_id: 2 }, Some(capture_start));
        assert!(entry.since_start < Duration::from_secs(5));
        assert!(entry.recorded_at >= UNIX_EPOCH);
    }
}
