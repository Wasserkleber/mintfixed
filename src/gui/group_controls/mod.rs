use super::*;
use crate::state::{ModData_v0_1_0 as Data, ModProfile_v0_1_0 as Profile};

mod drag_drop;
mod import_export;
mod management;

pub(super) use drag_drop::{
    Drag, GroupDestination, Target, add_files, auto_scroll, drop_files, hover_open, row_target,
};
pub(super) use management::{controls, remove_group_member};

fn attach(profile: &mut Profile, name: &str) {
    profile.mods.insert(
        0,
        ModOrGroup::Group {
            id: crate::state::new_id(),
            group_name: name.into(),
            enabled: true,
            member_overrides: Default::default(),
        },
    );
}
