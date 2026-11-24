//! Drag payloads, selection, movement, drop targets and scrolling.
use super::*;

mod auto_scroll;
mod file_drop;
mod model;
mod movement;
mod multi_select;
mod row_target;

pub(in crate::gui) use auto_scroll::{auto_scroll, hover_open};
pub(in crate::gui) use file_drop::{GroupDestination, add_files, drop_files};
pub(in crate::gui) use model::{Drag, Target};
pub(in crate::gui) use row_target::row_target;

use model::{DRAG_PAINT_ANCHOR_ID, DragPaintAnchor, drag_paint_translation};

#[cfg(test)]
use super::management::rename_group;
#[cfg(test)]
use auto_scroll::{HoverOpen, edge_scroll_delta};

#[cfg(test)]
#[path = "tests/drag_polish_tests.rs"]
mod drag_polish_tests;
#[cfg(test)]
#[path = "tests/move_switch_tests.rs"]
mod move_switch_tests;
#[cfg(test)]
#[path = "tests/multi_drag_tests.rs"]
mod multi_drag_tests;
