//! Source-control version resolution for the mintfixed build script.
//!
//! This module deliberately uses only the standard library and the local Git executable.
//! It does not contact GitHub and does not change the Cargo/package/update version.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildVersion {
    pub display: String,
    pub commit_sha: Option<String>,
    pub base_tag: Option<String>,
    pub commits_since_tag: Option<u64>,
    pub modified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReleaseNumber {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Option<String>,
}

impl ReleaseNumber {
    fn from_tag(tag: &str) -> Option<Self> {
        let value = tag.strip_prefix("mintfixed_")?;
        let (numeric, prerelease) = match value.split_once('-') {
            Some((numeric, suffix)) => (numeric, Some(suffix)),
            None => (value, None),
        };
        let parts: Vec<&str> = numeric.split('.').collect();
        if !(2..=3).contains(&parts.len()) {
            return None;
        }
        let numbers: Option<Vec<u64>> = parts.iter().map(|part| {
            if part.is_empty()
                || (part.len() > 1 && part.starts_with('0'))
                || !part.bytes().all(|b| b.is_ascii_digit())
            {
                return None;
            }
            part.parse().ok()
        }).collect();
        let numbers = numbers?;
        if let Some(suffix) = prerelease {
            if suffix.is_empty() || suffix.split('.').any(|item| {
                item.is_empty() || !item.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            }) {
                return None;
            }
        }
        Some(Self {
            major: numbers[0],
            minor: numbers[1],
            patch: *numbers.get(2).unwrap_or(&0),
            prerelease: prerelease.map(str::to_owned),
        })
    }

    fn display(&self) -> String {
        // Keep the two-component release scheme, e.g. `1.3`, from the tag.
        // The caller uses the original tag text, not this normalized value.
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug)]
struct Candidate {
    name: String,
    distance: u64,
    release: Option<ReleaseNumber>,
}

// Stable beats prerelease when the same distance is involved, followed by
// descending numeric version and SemVer-like prerelease identifiers.
fn candidate_order(a: &Candidate, b: &Candidate) -> Ordering {
    a.distance
        .cmp(&b.distance)
        .then_with(|| match (&a.release, &b.release) {
            (Some(x), Some(y)) => {
                x.prerelease.is_some().cmp(&y.prerelease.is_some())
                    .then_with(|| (y.major, y.minor, y.patch).cmp(&(x.major, x.minor, x.patch)))
                    .then_with(|| compare_prereleases(y.prerelease.as_deref(), x.prerelease.as_deref()))
            }
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
        .then_with(|| a.name.cmp(&b.name))
}

fn compare_prereleases(a: Option<&str>, b: Option<&str>) -> Ordering {
    let (Some(a), Some(b)) = (a, b) else { return Ordering::Equal; };
    for (a, b) in a.split('.').zip(b.split('.')) {
        let order = match (a.parse::<u64>(), b.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            _ => a.cmp(b),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    a.split('.').count().cmp(&b.split('.').count())
}

fn is_legacy_tag(tag: &str) -> bool {
    tag.strip_prefix("v0.2.10-mintfixed-")
        .is_some_and(|suffix| !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()))
}

fn git_output(working_dir: &Path, args: &[&str]) -> Option<Output> {
    Command::new("git").arg("-C").arg(working_dir).args(args).output().ok()
}

fn git_text(working_dir: &Path, args: &[&str]) -> Option<String> {
    let output = git_output(working_dir, args)?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn git_success(working_dir: &Path, args: &[&str]) -> bool {
    git_output(working_dir, args).is_some_and(|output| output.status.success())
}

fn git_repo_root(working_dir: &Path) -> Option<PathBuf> {
    git_text(working_dir, &["rev-parse", "--show-toplevel"])
        .filter(|result| !result.is_empty())
        .map(PathBuf::from)
}

fn short_sha(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

fn relevant_untracked(path: &str) -> bool {
    let path = Path::new(path);
    let is_source_area = path.starts_with("src")
        || path.starts_with("mint_lib")
        || path.starts_with("hook")
        || path.starts_with("hook_resolvers")
        || path.starts_with("assets")
        || path.starts_with("tests")
        || path.starts_with(".github/workflows");
    is_source_area && path.extension().and_then(|x| x.to_str()).is_some_and(|ext| {
        matches!(ext, "rs" | "toml" | "json" | "yml" | "yaml" | "svg" | "png" | "h" | "c" | "cpp" | "pak" | "vert" | "frag")
    })
}

fn is_modified(repo: &Path) -> Result<bool, String> {
    let output = git_output(repo, &["status", "--porcelain=v1", "--untracked-files=all", "-z"])
        .filter(|out| out.status.success())
        .ok_or_else(|| "failed to inspect Git working tree".to_owned())?;
    // Porcelain -z is NUL delimited, so spaces/newlines in file names are safe.
    for record in output.stdout.split(|b| *b == 0) {
        if record.len() < 4 || record[2] != b' ' {
            continue; // The second path of a rename/copy is not a status entry.
        }
        if &record[..2] == b"??" {
            if relevant_untracked(&String::from_utf8_lossy(&record[3..])) {
                return Ok(true);
            }
        } else if &record[..2] != b"!!" {
            return Ok(true);
        }
    }
    Ok(false)
}

fn collect_candidates(repo: &Path, sha: &str, shallow: bool) -> Vec<Candidate> {
    let tags = git_text(repo, &["for-each-ref", "--format=%(refname:strip=2)", "refs/tags"])
        .unwrap_or_default();
    let mut candidates = Vec::new();
    for name in tags.lines() {
        let release = ReleaseNumber::from_tag(name);
        if release.is_none() && !is_legacy_tag(name) {
            continue;
        }
        // ^{commit} peels both annotated and lightweight tags.
        let Some(commit) = git_text(repo, &["rev-parse", "--verify", &format!("refs/tags/{name}^{{commit}}")]) else {
            continue;
        };
        if !git_success(repo, &["merge-base", "--is-ancestor", &commit, sha]) {
            continue;
        }
        // In a shallow clone the reported distance may be truncated. Only an
        // exact tag is meaningful without the complete commit graph.
        if shallow && commit != sha {
            continue;
        }
        let Some(distance) = git_text(repo, &["rev-list", "--count", &format!("{commit}..{sha}")])
            .and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        candidates.push(Candidate { name: name.to_owned(), distance, release });
    }
    candidates
}

fn fallback(cargo_version: &str) -> BuildVersion {
    BuildVersion {
        display: format!("mintfixed {cargo_version} (version metadata unavailable)"),
        commit_sha: None,
        base_tag: None,
        commits_since_tag: None,
        modified: false,
    }
}

/// Resolve from the checked-out source. An optional release context is *validated*,
/// not trusted as a version override; a mismatch fails the build.
pub fn resolve(working_dir: &Path, cargo_version: &str, release_tag: Option<&str>) -> Result<BuildVersion, String> {
    let Some(repo) = git_repo_root(working_dir) else {
        return if release_tag.is_some() {
            Err("release build requires a complete Git checkout and an available Git executable".to_owned())
        } else {
            Ok(fallback(cargo_version))
        };
    };
    let sha = match git_text(&repo, &["rev-parse", "--verify", "HEAD^{commit}"]) {
        Some(sha) if sha.len() >= 7 => sha,
        _ if release_tag.is_none() => return Ok(fallback(cargo_version)),
        _ => return Err("release build could not resolve HEAD".to_owned()),
    };
    let shallow = git_text(&repo, &["rev-parse", "--is-shallow-repository"])
        .as_deref() == Some("true");
    let modified = is_modified(&repo)?;
    let mut candidates = collect_candidates(&repo, &sha, shallow);
    let chosen = if let Some(tag) = release_tag {
        if ReleaseNumber::from_tag(tag).is_none() && !is_legacy_tag(tag) {
            return Err(format!("invalid mintfixed release tag: {tag:?}"));
        }
        if shallow {
            return Err("release builds require full Git history (checkout fetch-depth: 0)".to_owned());
        }
        if modified {
            return Err("refusing to build a dirty official release".to_owned());
        }
        let tag_sha = git_text(&repo, &["rev-parse", "--verify", &format!("refs/tags/{tag}^{{commit}}")])
            .ok_or_else(|| format!("release tag {tag:?} was not fetched"))?;
        if tag_sha != sha {
            return Err(format!("release tag {tag:?} resolves to {tag_sha}, but HEAD is {sha}"));
        }
        Some(Candidate {
            name: tag.to_owned(), distance: 0,
            release: ReleaseNumber::from_tag(tag),
        })
    } else {
        // Never use legacy numbering if a new-style ancestor exists.
        if candidates.iter().any(|c| c.release.is_some()) {
            candidates.retain(|c| c.release.is_some());
        }
        candidates.sort_by(candidate_order);
        candidates.into_iter().next()
    };
    let short = short_sha(&sha);
    let (display, tag, distance) = match chosen {
        Some(candidate) => {
            let label = if let Some(release) = &candidate.release {
                // `mintfixed_1.3` -> `v1.3`; leave prerelease spelling intact.
                let _ = release.display();
                format!("v{}", candidate.name.strip_prefix("mintfixed_").unwrap())
            } else {
                candidate.name.clone() // legacy name, deliberately not re-numbered
            };
            let mut display = format!("mintfixed {label}");
            if candidate.distance > 0 {
                display.push_str(&format!(" +{} commits", candidate.distance));
            }
            if candidate.distance > 0 || modified {
                display.push_str(&format!(" ({short}"));
                if modified { display.push_str(", modified"); }
                display.push(')');
            }
            (display, Some(candidate.name), Some(candidate.distance))
        }
        None => (format!("mintfixed dev ({short}{})", if modified { ", modified" } else { "" }), None, None),
    };
    Ok(BuildVersion { display, commit_sha: Some(sha), base_tag: tag, commits_since_tag: distance, modified })
}

/// Watch Git's actual paths: works in linked worktrees and detached checkouts.
/// `built` may emit its own directives; these add the missing Git invalidation.
pub fn emit_rerun_directives(working_dir: &Path) {
    println!("cargo:rerun-if-env-changed=MINTFIXED_RELEASE_TAG");
    println!("cargo:rerun-if-changed=build_version.rs");
    println!("cargo:rerun-if-changed=build.rs");
    let Some(repo) = git_repo_root(working_dir) else {
        return;
    };

    // Watching tracked files keeps unstaged tracked changes visible to Cargo.
    // Watch source directories separately to detect new, not-yet-tracked files.
    if let Some(output) = git_output(&repo, &["ls-files", "--cached", "-z"]) {
        if output.status.success() {
            for file in output.stdout.split(|b| *b == 0).filter(|x| !x.is_empty()) {
                let path = repo.join(String::from_utf8_lossy(file).as_ref());
                watch(&path);
            }
        }
    }
    for dir in ["src", "mint_lib/src", "mint_lib/tests", "hook/src", "hook_resolvers/src", "assets", "tests", ".github/workflows"] {
        let path = repo.join(dir);
        if path.is_dir() { watch(&path); }
    }
    for relative_git_path in ["HEAD", "index", "packed-refs", "refs", "refs/heads", "refs/tags"] {
        watch_git_path(&repo, relative_git_path);
    }
    if let Some(current_ref) = git_text(&repo, &["symbolic-ref", "-q", "HEAD"]) {
        watch_git_path(&repo, &current_ref);
    }
    // Cargo also needs to re-run when a linked worktree's .git pointer changes.
    let git_pointer = repo.join(".git");
    if git_pointer.is_file() { watch(&git_pointer); }
}

fn watch_git_path(repo: &Path, relative: &str) {
    if let Some(git_path) = git_text(repo, &["rev-parse", "--git-path", relative]) {
        let path = PathBuf::from(git_path);
        let path = if path.is_absolute() { path } else { repo.join(path) };
        // Nonexistent refs/packed-refs are still watched to catch creation.
        watch(&path);
    }
}

fn watch(path: &Path) {
    if let Some(path) = path.to_str() {
        if !path.contains('\n') { println!("cargo:rerun-if-changed={path}"); }
    }
}

fn rust_option(value: Option<&str>) -> String {
    value.map(|value| format!("Some({value:?})")).unwrap_or_else(|| "None".to_owned())
}

pub fn write_generated(out_dir: &Path, info: &BuildVersion) -> std::io::Result<()> {
    let contents = format!(
        "// Generated by mint_lib/build.rs; do not edit.\n\
         pub const DISPLAY_VERSION: &str = {:?};\n\
         pub const COMMIT_SHA: Option<&str> = {};\n\
         pub const BASE_TAG: Option<&str> = {};\n\
         pub const COMMITS_SINCE_TAG: Option<u64> = {};\n\
         pub const MODIFIED: bool = {};\n",
        info.display,
        rust_option(info.commit_sha.as_deref()),
        rust_option(info.base_tag.as_deref()),
        info.commits_since_tag.map(|d| format!("Some({d})")).unwrap_or_else(|| "None".to_owned()),
        info.modified,
    );
    let path = out_dir.join("mintfixed_version.rs");
    if fs::read_to_string(&path).ok().as_deref() != Some(&contents) {
        fs::write(path, contents)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Repo(PathBuf);
    impl Repo {
        fn new() -> Self {
            let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            let path = env::temp_dir().join(format!("mintfixed-version-{}-{unique}", std::process::id()));
            fs::create_dir_all(path.join("src")).unwrap();
            let repo = Self(path);
            repo.git(&["init", "-q"]);
            repo.git(&["config", "user.name", "Mint Test"]);
            repo.git(&["config", "user.email", "test@mintfixed.invalid"]);
            fs::write(repo.0.join("src/main.rs"), "fn main() {}\n").unwrap();
            fs::write(repo.0.join(".gitignore"), "target/\n").unwrap();
            repo.git(&["add", "."]);
            repo.git(&["commit", "-q", "-m", "initial"]);
            repo
        }
        fn git(&self, args: &[&str]) -> String {
            let output = git_output(&self.0, args).expect("Git must exist for resolver tests");
            assert!(output.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&output.stderr));
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }
        fn commit(&self, n: usize) {
            fs::write(self.0.join("src/main.rs"), format!("fn main() {{ let _ = {n}; }}\n")).unwrap();
            self.git(&["add", "src/main.rs"]);
            self.git(&["commit", "-q", "-m", &format!("commit-{n}")]);
        }
        fn version(&self) -> BuildVersion { resolve(&self.0, "0.2.10", None).unwrap() }
    }
    impl Drop for Repo {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn exact_stable_beta_annotated_and_commit_distance() {
        let repo = Repo::new();
        repo.git(&["tag", "mintfixed_1.3"]);
        assert_eq!(repo.version().display, "mintfixed v1.3");
        for i in 1..=4 { repo.commit(i); }
        let sha = repo.git(&["rev-parse", "--short=7", "HEAD"]);
        assert_eq!(repo.version().display, format!("mintfixed v1.3 +4 commits ({sha})"));
        repo.git(&["tag", "-a", "mintfixed_1.4-beta.1", "-m", "beta"]);
        assert_eq!(repo.version().display, "mintfixed v1.4-beta.1");
        repo.commit(5);
        assert_eq!(repo.version().display, format!("mintfixed v1.4-beta.1 +1 commits ({})", short_sha(&repo.git(&["rev-parse", "HEAD"]))));
    }

    #[test]
    fn stable_and_numeric_tags_win_deterministically() {
        let repo = Repo::new();
        repo.git(&["tag", "mintfixed_1.2"]);
        repo.git(&["tag", "mintfixed_1.10"]);
        assert_eq!(repo.version().display, "mintfixed v1.10");
        repo.git(&["tag", "mintfixed_2.0-beta.1"]);
        assert_eq!(repo.version().display, "mintfixed v1.10");
        repo.git(&["tag", "mintfixed_2.0"]);
        assert_eq!(repo.version().display, "mintfixed v2.0");
    }

    #[test]
    fn unrelated_branch_tag_and_legacy_fallback() {
        let repo = Repo::new();
        let main = repo.git(&["branch", "--show-current"]);
        repo.git(&["switch", "-q", "-c", "other"]);
        repo.commit(1);
        repo.git(&["tag", "mintfixed_9.9"]);
        repo.git(&["switch", "-q", &main]);
        repo.git(&["tag", "v0.2.10-mintfixed-1"]);
        assert_eq!(repo.version().display, "mintfixed v0.2.10-mintfixed-1");
        repo.git(&["tag", "mintfixed_1.3"]);
        assert_eq!(repo.version().display, "mintfixed v1.3");
    }

    #[test]
    fn tracked_untracked_ignored_and_untagged() {
        let repo = Repo::new();
        assert!(repo.version().display.starts_with("mintfixed dev ("));
        repo.git(&["tag", "mintfixed_1.3"]);
        fs::create_dir_all(repo.0.join("target")).unwrap();
        fs::write(repo.0.join("target/output.exe"), "ignored").unwrap();
        assert_eq!(repo.version().display, "mintfixed v1.3");
        fs::write(repo.0.join("src/new.rs"), "// new source").unwrap();
        assert!(repo.version().display.contains(", modified)"));
        fs::remove_file(repo.0.join("src/new.rs")).unwrap();
        fs::write(repo.0.join("src/main.rs"), "// changed tracked source").unwrap();
        assert!(repo.version().display.contains(", modified)"));
    }

    #[test]
    fn validates_release_context_and_dirty_check() {
        let repo = Repo::new();
        repo.git(&["tag", "mintfixed_1.3"]);
        assert_eq!(resolve(&repo.0, "0.2.10", Some("mintfixed_1.3")).unwrap().display, "mintfixed v1.3");
        assert!(resolve(&repo.0, "0.2.10", Some("mintfixed_1.2")).is_err());
        repo.commit(1);
        assert!(resolve(&repo.0, "0.2.10", Some("mintfixed_1.3")).is_err());
        repo.git(&["tag", "mintfixed_1.4-beta.1"]);
        fs::write(repo.0.join("src/new.rs"), "// dirty").unwrap();
        assert!(resolve(&repo.0, "0.2.10", Some("mintfixed_1.4-beta.1")).is_err());
    }

    #[test]
    fn without_git_uses_cargo_fallback() {
        let path = env::temp_dir().join(format!("mintfixed-no-git-{}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        assert_eq!(resolve(&path, "0.2.10", None).unwrap().display,
            "mintfixed 0.2.10 (version metadata unavailable)");
        assert!(resolve(&path, "0.2.10", Some("mintfixed_1.3")).is_err());
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn linked_worktree_and_detached_head() {
        let repo = Repo::new();
        repo.git(&["tag", "mintfixed_1.3"]);
        let worktree = repo.0.with_extension("linked");
        repo.git(&["worktree", "add", "-q", "--detach", worktree.to_str().unwrap()]);
        assert_eq!(resolve(&worktree, "0.2.10", None).unwrap().display, "mintfixed v1.3");
        repo.git(&["worktree", "remove", "--force", worktree.to_str().unwrap()]);
    }

    #[test]
    fn tag_validation_and_generated_escaping() {
        for bad in ["mintfixed_1", "mintfixed_1.02", "mintfixed_1.3-", "mintfixed_1.3-evil\n", "mintfixed_1.3+meta"] {
            assert!(ReleaseNumber::from_tag(bad).is_none(), "{bad}");
        }
        assert!(ReleaseNumber::from_tag("mintfixed_1.4-beta.1").is_some());
        let path = env::temp_dir().join(format!("mintfixed-escape-{}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        write_generated(&path, &BuildVersion { display: "test \"name\"\\path".into(), commit_sha: None, base_tag: None, commits_since_tag: None, modified: false }).unwrap();
        let text = fs::read_to_string(path.join("mintfixed_version.rs")).unwrap();
        assert!(text.contains(r#"test \"name\"\\path"#));
        let _ = fs::remove_dir_all(path);
    }
}