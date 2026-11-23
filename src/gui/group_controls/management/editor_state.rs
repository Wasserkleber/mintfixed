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

/// A background export/import: empty until the worker stores its result.
type Job<T> = std::sync::Arc<std::sync::Mutex<Option<Result<T, String>>>>;

#[derive(Clone, Default)]
struct ExportEditor {
    selected: String,
    kind: GroupExportKind,
    include_files: bool,
    job: Option<Job<std::path::PathBuf>>,
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
    job: Option<Job<(String, crate::state::ModGroup)>>,
    name: String,
    success: Option<String>,
    error: Option<String>,
}

