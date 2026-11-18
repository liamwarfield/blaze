//! Probe: the `EMPTY_PLATFORM_INFO` fingerprint. A `SymlinkAction` created with
//! `toArtifact` (no input path) hashes only its GUID, so its key isolates
//! `ActionKeyComputer.getKey`'s platform part.

mod oracle_common;

use std::any::Any;

use blaze_actions::actions::action::{
    AbstractAction, Action, ComputeKeyError, InputMetadataProvider,
};
use blaze_actions::{ActionKeyContext, ActionOwner};
use blaze_collect::NestedSetBuilder;
use blaze_util::Fingerprint;
use oracle_common::Recording;

#[derive(Debug)]
struct GuidOnly {
    base: AbstractAction,
    guid: &'static str,
}

impl Action for GuidOnly {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn get_mnemonic(&self) -> String {
        "Symlink".to_string()
    }
    fn compute_key(
        &self,
        _ctx: &ActionKeyContext,
        _imp: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string(self.guid);
        Ok(())
    }
    // Java: SymlinkAction.getExecutionPlatform / getExecProperties
    fn get_execution_platform(&self) -> Option<blaze_actions::PlatformInfoRef> {
        Some(blaze_actions::actions::platform_info::empty_platform_info())
    }
    fn get_exec_properties(&self) -> indexmap::IndexMap<String, String> {
        indexmap::IndexMap::new()
    }
}

#[test]
fn empty_platform_fingerprint_matches_oracle_symlink_key() {
    let Some(rec) = Recording::load("brotli") else {
        eprintln!("no oracle recording; skipped");
        return;
    };
    let action = rec
        .actions()
        .iter()
        .find(|a| a["mnemonic"] == "Symlink")
        .unwrap();
    let owner: ActionOwner = rec.action_owner(action);
    let a = GuidOnly {
        base: AbstractAction::new(
            owner,
            NestedSetBuilder::wrap(
                blaze_collect::Order::STABLE_ORDER,
                rec.flattened_inputs(action),
            ),
            rec.outputs(action),
        ),
        // Java: SymlinkAction.GUID
        guid: "7f4fab4d-d0a7-4f0f-8649-1d0337a21fee",
    };
    let key = a.get_key(&ActionKeyContext::new(), None).unwrap();
    assert_eq!(key, action["actionKey"].as_str().unwrap());
}

#[derive(Debug)]
struct EmptyFileWrite {
    base: AbstractAction,
}

impl Action for EmptyFileWrite {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn get_mnemonic(&self) -> String {
        "FileWrite".to_string()
    }
    fn compute_key(
        &self,
        _ctx: &ActionKeyContext,
        _imp: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        // Java: FileWriteAction.RegularFileWriteAction.computeKey
        fp.add_string("332877c7-ca9f-4731-b387-54f620408522")
            .add_boolean(false)
            .add_string("");
        Ok(())
    }
    fn get_exec_properties(&self) -> indexmap::IndexMap<String, String> {
        indexmap::IndexMap::new()
    }
}

#[test]
fn host_platform_fingerprint_matches_oracle_file_write_key() {
    let Some(rec) = Recording::load("brotli") else {
        eprintln!("no oracle recording; skipped");
        return;
    };
    let action = rec
        .actions()
        .iter()
        .find(|a| a["mnemonic"] == "FileWrite")
        .unwrap();
    let a = EmptyFileWrite {
        base: AbstractAction::new(
            rec.action_owner(action),
            NestedSetBuilder::empty_set(blaze_collect::Order::STABLE_ORDER),
            rec.outputs(action),
        ),
    };
    let key = a.get_key(&ActionKeyContext::new(), None).unwrap();
    assert_eq!(key, action["actionKey"].as_str().unwrap());
}
