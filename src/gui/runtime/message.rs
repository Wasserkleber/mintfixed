use std::collections::BTreeSet;
use std::ops::{Deref, DerefMut};
use std::time::SystemTime;
use std::{collections::HashMap, sync::Arc};

use snafu::prelude::*;
use tokio::{
    sync::mpsc::{self, Sender},
    task::JoinHandle,
};
use tracing::*;

use crate::gui::SelfUpdateProgress;
use crate::gui::{
    App, SpecFetchProgress, WindowProviderParameters,
    request_counter::{RequestCounter, RequestID},
};
use crate::gui::LastAction;
use crate::integrate::*;
use crate::mod_lints::{LintId, LintReport};
use crate::state::{ModData_v0_1_0 as ModData, ModOrGroup};
use crate::*;
use crate::{
    providers::{FetchProgress, ModInfo, ModStore},
    state::ModConfig,
};
use mint_lib::error::GenericError;
use mint_lib::mod_info::MetaConfig;
use mint_lib::update::GitHubRelease;

mod dispatch;
mod resolve;
mod integrate;
mod cache;
mod check_updates;
mod lint;
mod self_update;

pub(in crate::gui) use dispatch::{InstallPhase, Message, MessageHandle};
pub(in crate::gui) use resolve::ResolveMods;
pub(in crate::gui) use integrate::{Integrate, FetchModProgress};
pub(in crate::gui) use cache::UpdateCache;
pub(in crate::gui) use check_updates::CheckUpdates;
pub(in crate::gui) use lint::LintMods;
pub(in crate::gui) use self_update::{SelfUpdate, FetchSelfUpdateProgress};
use check_updates::integrate_async;
