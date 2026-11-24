//! Shared group management operations, popup sections and editor state.
use super::import_export::*;
use super::*;

mod controls;
mod creation;
mod delete;
mod duplicate;
mod editor_state;
mod export;
mod import;
mod jobs;
mod operations;
mod picker;
mod rename;

pub(in crate::gui) use controls::controls;
pub(in crate::gui) use operations::remove_group_member;
pub(super) use operations::rename_group;

use creation::show_creation_and_attachment;
use delete::show_delete;
use duplicate::show_duplicate;
use editor_state::{DeleteEditor, DuplicateEditor, ExportEditor, GroupExportKind, ImportEditor, RenameEditor};
use export::show_export;
use import::show_import;
use jobs::poll_package_jobs;
use operations::{delete_group, duplicate_group};
use picker::{group_action_button, group_option_row, group_picker_button, group_picker_scroll};
use rename::show_rename;

#[cfg(test)]
#[path = "tests/rename_tests.rs"]
mod rename_tests;
