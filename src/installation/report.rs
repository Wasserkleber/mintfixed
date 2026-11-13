//! Successful-install counts and skipped occurrences, captured before deduplication.
#[derive(Debug, Default)]
pub(crate) struct InstallReport {
    pub installed: usize,
    pub skipped: Vec<SkippedMod>,
}

#[derive(Debug)]
pub(crate) struct SkippedMod {
    pub name: String,
    pub location: String,
    pub reason: &'static str,
}

impl InstallReport {
    pub fn summary(&self) -> String {
        let mods = if self.installed == 1 { "mod" } else { "mods" };
        let duplicates = if self.skipped.len() == 1 {
            "duplicate"
        } else {
            "duplicates"
        };
        format!(
            "Installed {} {mods}. Skipped {} {duplicates}.",
            self.installed,
            self.skipped.len()
        )
    }

    pub fn details(&self) -> String {
        let mut lines = vec![self.summary()];
        for entry in &self.skipped {
            lines.push(format!(
                "Skipped {:?} at {}: {}.",
                entry.name, entry.location, entry.reason
            ));
        }
        lines.join("\n")
    }
}
