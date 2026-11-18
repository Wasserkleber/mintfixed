#[derive(Clone, Default)]
struct RenameEditor {
    selected: String,
    name: String,
    error: Option<String>,
    success: Option<String>,
    picker_open: bool,
}

#[derive(Clone, Default)]
struct DeleteEditor {
    selected: String,
    picker_open: bool,
    confirming: bool,
    error: Option<String>,
    success: Option<String>,
}

#[derive(Clone, Default)]
struct DuplicateEditor {
    selected: String,
    name: String,
    picker_open: bool,
    error: Option<String>,
    success: Option<String>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum GroupExportKind {
    #[default]
    Json,
    Txt,
}

impl GroupExportKind {
    fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Txt => "txt",
        }
    }
}

#[derive(Clone, Default)]
struct ExportEditor {
    selected: String,
    kind: GroupExportKind,
    include_files: bool,
    job: Option<std::sync::Arc<std::sync::Mutex<Option<Result<std::path::PathBuf, String>>>>>,
    picker_open: bool,
    success: Option<String>,
    error: Option<String>,
}

#[derive(Clone)]
pub(super) struct PendingGroupImport {
    pub(super) suggested_name: Option<String>,
    pub(super) group: crate::state::ModGroup,
}

#[derive(Clone, Default)]
struct ImportEditor {
    pending: Option<PendingGroupImport>,
    bundle_path: Option<std::path::PathBuf>,
    job: Option<std::sync::Arc<std::sync::Mutex<Option<Result<(String, crate::state::ModGroup), String>>>>>,
    name: String,
    success: Option<String>,
    error: Option<String>,
}

