// Mechanical source extraction from src/integrate.rs; no algorithm changes.
use super::*;

pub(super) fn collect_dir_files(
    dir: &'static include_dir::Dir,
    collect: &mut HashMap<String, &[u8]>,
) {
    for entry in dir.entries() {
        match entry {
            include_dir::DirEntry::Dir(dir) => {
                collect_dir_files(dir, collect);
            }
            include_dir::DirEntry::File(file) => {
                collect.insert(
                    file.path().to_str().unwrap().replace('\\', "/"),
                    file.contents(),
                );
            }
        }
    }
}

pub(super) fn format_soft_class<P: AsRef<PakPath>>(path: P) -> String {
    let path = path.as_ref();
    let name = path.file_stem().unwrap();
    format!(
        "/Game/{}{}_C",
        path.strip_prefix("FSD/Content")
            .unwrap()
            .as_str()
            .strip_suffix("uasset")
            .unwrap(),
        name
    )
}
