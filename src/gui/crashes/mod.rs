//! crashes GUI modules.
pub(in crate::gui) mod crash_history;
pub(in crate::gui) mod crash_notice;
#[cfg(all(feature = "hook", target_os = "windows"))]
pub(in crate::gui) mod crash_reporter_update;
