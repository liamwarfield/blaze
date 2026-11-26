//! Extra actions: ports of `lib/analysis/extra/` (`ExtraAction`, `ExtraActionSpec`,
//! `ExtraActionInfoFileWriteAction`, `ExtraActionMapProvider`), of `analysis/ExtraActionUtils.java`
//! with `analysis/ExtraActionsVisitor.java`, and of `analysis/PseudoAction.java`.
//!
//! The rules (`action_listener`, `extra_action`) are in `blaze_rules::extra`; the execution of
//! the actions (`ExecutableAction`) in [`execution`].

pub mod execution;
pub mod extra_action;
pub mod extra_action_info_file_write_action;
pub mod extra_action_map_provider;
pub mod extra_action_spec;
pub mod extra_action_utils;
pub mod pseudo_action;

#[cfg(test)]
mod extra_action_test;
