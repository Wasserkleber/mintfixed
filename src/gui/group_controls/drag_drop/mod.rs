//! Drag payloads, selection, movement, drop targets and scrolling.
use super::*;

mod model;
mod multi_select;
mod movement;
mod row_target;
mod file_drop;
mod auto_scroll;

pub(in crate::gui) use model::{Drag, Target};
pub(in crate::gui) use row_target::row_target;
pub(in crate::gui) use file_drop::{GroupDestination, add_files, drop_files};
pub(in crate::gui) use auto_scroll::{auto_scroll, hover_open};

use model::{DragPaintAnchor, DRAG_PAINT_ANCHOR_ID, drag_paint_translation};

#[cfg(test)]
use auto_scroll::{HoverOpen, edge_scroll_delta};
#[cfg(test)]
use super::management::rename_group;

#[cfg(test)]
#[path = "tests/drag_polish_tests.rs"]
mod drag_polish_tests;
#[cfg(test)]
#[path = "tests/move_switch_tests.rs"]
mod move_switch_tests;
#[cfg(test)]
#[path = "tests/multi_drag_tests.rs"]
mod multi_drag_tests;
