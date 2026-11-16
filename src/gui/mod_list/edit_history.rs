//! Undo/redo for saved edits to the mod list (not Group Management actions).
//! Each step stores the entire profile/group state, including shared member overrides,
//! stable IDs and ordering, so cross-group moves restore all affected profiles.
use crate::state::config::ConfigWrapper;
use crate::state::{ModData_v0_1_0 as ModData, VersionAnnotatedModData};
use std::ops::DerefMut;

const MAX_HISTORY: usize = 64;

pub(super) struct ModEditHistory {
    undo: Vec<ModData>,
    redo: Vec<ModData>,
    baseline: ModData,
    revision: u64,
}

impl ModEditHistory {
    pub fn new(data: &ModData, revision: u64) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            baseline: data.clone(),
            revision,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Only inspect the mod list after its save revision changes. Idle UI frames
    /// do no cloning, serialization or history allocations.
    pub fn sync(&mut self, data: &ModData, revision: u64) {
        if revision == self.revision {
            return;
        }
        self.revision = revision;
        // A save may happen without any real change. Do not invalidate redo.
        if serde_json::to_vec(&self.baseline).expect("serializable mod data")
            == serde_json::to_vec(data).expect("serializable mod data")
        {
            return;
        }
        let previous = std::mem::replace(&mut self.baseline, data.clone());
        push_limited(&mut self.undo, previous);
        self.redo.clear();
    }

    /// Settings/Group Management/profile management changes are explicitly not
    /// undoable. Invalidate old snapshots: replaying them could resurrect
    /// deleted shared groups or overwrite renamed profiles.
    pub fn reset(&mut self, data: &ModData, revision: u64) {
        self.undo.clear();
        self.redo.clear();
        self.baseline = data.clone();
        self.revision = revision;
    }

    /// Commit history only after the restored data has been saved. On an I/O
    /// failure, restore the in-memory state and keep both history stacks intact.
    pub fn restore(
        &mut self,
        data: &mut ConfigWrapper<VersionAnnotatedModData>,
        redo: bool,
    ) -> Result<bool, String> {
        let target = if redo {
            self.redo.last()
        } else {
            self.undo.last()
        };
        let Some(target) = target.cloned() else {
            return Ok(false);
        };
        let previous = (***data).clone();
        *data.deref_mut().deref_mut() = target.clone();
        if let Err(error) = data.save() {
            *data.deref_mut().deref_mut() = previous;
            return Err(format!("Could not save restored mod list: {error}"));
        }
        if redo {
            self.redo.pop();
            push_limited(&mut self.undo, previous);
        } else {
            self.undo.pop();
            push_limited(&mut self.redo, previous);
        }
        self.baseline = target;
        self.revision = data.revision();
        Ok(true)
    }
}

fn push_limited(stack: &mut Vec<ModData>, item: ModData) {
    if stack.len() == MAX_HISTORY {
        stack.remove(0);
    }
    stack.push(item);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiple_undo_redo_and_invalidate_redo_after_new_edit() {
        let mut data = ModData::default();
        let mut history = ModEditHistory::new(&data, 0);
        data.profiles.insert("profile1".into(), Default::default());
        history.sync(&data, 1);
        data.profiles.insert("profile2".into(), Default::default());
        history.sync(&data, 2);
        let mut config = ConfigWrapper::memory(VersionAnnotatedModData::V0_3_0(data));
        assert!(history.restore(&mut config, false).unwrap());
        assert!(config.profiles.contains_key("profile1"));
        assert!(!config.profiles.contains_key("profile2"));
        assert!(history.restore(&mut config, false).unwrap());
        assert!(!config.profiles.contains_key("profile1"));
        assert!(history.restore(&mut config, true).unwrap());
        assert!(config.profiles.contains_key("profile1"));
        assert!(history.can_redo());
        config
            .profiles
            .insert("new_edit".into(), Default::default());
        config.save().unwrap();
        history.sync(&config, config.revision());
        assert!(!history.can_redo());
    }

    #[test]
    fn unchanged_save_preserves_redo_and_reset_clears_history() {
        let mut data = ModData::default();
        let mut history = ModEditHistory::new(&data, 0);
        data.profiles.insert("temporary".into(), Default::default());
        history.sync(&data, 1);
        let mut config = ConfigWrapper::memory(VersionAnnotatedModData::V0_3_0(data));
        history.restore(&mut config, false).unwrap();
        config.save().unwrap();
        history.sync(&config, config.revision());
        assert!(history.can_redo());
        history.reset(&config, config.revision());
        assert!(!history.can_undo());
        assert!(!history.can_redo());
    }
}
