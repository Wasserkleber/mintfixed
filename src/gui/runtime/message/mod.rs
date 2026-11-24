//! Background request dispatch and task-specific handlers.
mod dispatch;
mod resolve;
mod integrate;
mod cache;
mod check_updates;
mod lint;
mod self_update;

pub(in crate::gui) use dispatch::{InstallPhase, Message, MessageHandle};
pub(in crate::gui) use resolve::ResolveMods;
pub(in crate::gui) use integrate::{Integrate, FetchModProgress};
pub(in crate::gui) use cache::UpdateCache;
pub(in crate::gui) use check_updates::CheckUpdates;
pub(in crate::gui) use lint::LintMods;
pub(in crate::gui) use self_update::{SelfUpdate, FetchSelfUpdateProgress};
