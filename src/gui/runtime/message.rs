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

include!("message/dispatch.rs");
include!("message/resolve.rs");
include!("message/integrate.rs");
include!("message/cache.rs");
include!("message/check_updates.rs");
include!("message/lint.rs");
include!("message/self_update.rs");
