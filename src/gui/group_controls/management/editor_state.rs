#[derive(Clone, Default)]
pub(super) struct RenameEditor {
    pub(super) selected: String,
    pub(super) name: String,
    pub(super) error: Option<String>,
    pub(super) success: Option<String>,
    pub(super) picker_open: bool,
}

#[derive(Clone, Default)]
pub(super) struct DeleteEditor {
    pub(super) selected: String,
    pub(super) picker_open: bool,
    pub(super) confirming: bool,
    pub(super) error: Option<String>,
    pub(super) success: Option<String>,
}

#[derive(Clone, Default)]
pub(super) struct DuplicateEditor {
    pub(super) selected: String,
    pub(super) name: String,
    pub(super) picker_open: bool,
    pub(super) error: Option<String>,
    pub(super) success: Option<String>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum GroupExportKind {
    #[default]
    Json,
    Txt,
}

impl GroupExportKind {
    pub(super) fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Txt => "txt",
        }
    }
}

/// A background export/import: empty until the worker stores its result.
pub(super) type Job<T> = std::sync::Arc<std::sync::Mutex<Option<Result<T, String>>>>;

#[derive(Clone, Default)]
pub(super) struct ExportEditor {
    pub(super) selected: String,
    pub(super) kind: GroupExportKind,
    pub(super) include_files: bool,
    pub(super) job: Option<Job<std::path::PathBuf>>,
    pub(super) picker_open: bool,
    pub(super) success: Option<String>,
    pub(super) error: Option<String>,
}

#[derive(Clone)]
pub(in crate::gui::group_controls) struct PendingGroupImport {
    pub(in crate::gui::group_controls) suggested_name: Option<String>,
    pub(in crate::gui::group_controls) group: crate::state::ModGroup,
}

#[derive(Clone, Default)]
pub(super) struct ImportEditor {
    pub(super) pending: Option<PendingGroupImport>,
    pub(super) bundle_path: Option<std::path::PathBuf>,
    pub(super) job: Option<Job<(String, crate::state::ModGroup)>>,
    pub(super) name: String,
    pub(super) success: Option<String>,
    pub(super) error: Option<String>,
}
