//! Port of `analysis/PseudoAction.java`.

use std::any::Any;

use blaze_actions::actions::action::{
    AbstractAction, Action, CommandLineError, ComputeKeyError, InputMetadataProvider,
};
use blaze_actions::{ActionKeyContext, ActionOwner, Artifact};
use blaze_collect::NestedSet;
use blaze_protos::JavaMessage;
use blaze_protos::blaze::{ExtraActionInfo, PythonInfo, SpawnInfo};
use blaze_util::Fingerprint;
use blaze_vfs::PathFragment;

use crate::rule::RuleContext;

/// The extension of `ExtraActionInfo` a pseudo action sets and its value (Java's
/// `Extension<ExtraActionInfo, InfoType> infoExtension` and `InfoType info`).
#[derive(Clone, Debug, PartialEq)]
pub enum PseudoActionInfo {
    /// `SpawnInfo.spawnInfo` (`StarlarkActionFactory.SPAWN_INFO`).
    Spawn(SpawnInfo),
    /// `PythonInfo.pythonInfo` (`PyCommon.PYTHON_INFO`).
    Python(PythonInfo),
}

impl PseudoActionInfo {
    /// `getInfo().toByteArray()`.
    fn to_byte_array(&self) -> Vec<u8> {
        match self {
            PseudoActionInfo::Spawn(info) => info.to_byte_array(),
            PseudoActionInfo::Python(info) => info.to_byte_array(),
        }
    }

    /// `builder.setExtension(infoExtension, info)`.
    fn set_on(&self, builder: &mut ExtraActionInfo) {
        match self {
            PseudoActionInfo::Spawn(info) => builder.spawn_info = Some(info.clone()),
            PseudoActionInfo::Python(info) => builder.python_info = Some(info.clone()),
        }
    }
}

/// An action that is inserted into the build graph only to provide info about rules to
/// extra_actions.
// Java: com/google/devtools/build/lib/analysis/PseudoAction.java:44 PseudoAction
#[derive(Debug)]
pub struct PseudoAction {
    base: AbstractAction,
    /// `uuid`: (least significant bits, most significant bits).
    uuid: (i64, i64),
    mnemonic: String,
    info: PseudoActionInfo,
}

impl PseudoAction {
    /// `uuid` is `UUID.fromString(s)` of a canonical UUID string.
    // Java: com/google/devtools/build/lib/analysis/PseudoAction.java:52 PseudoAction.<init>
    pub fn new(
        uuid: &str,
        owner: ActionOwner,
        inputs: NestedSet<Artifact>,
        outputs: Vec<Artifact>,
        mnemonic: String,
        info: PseudoActionInfo,
    ) -> PseudoAction {
        PseudoAction {
            base: AbstractAction::new(owner, inputs, outputs),
            uuid: parse_uuid(uuid),
            mnemonic,
            info,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/PseudoAction.java:95 PseudoAction.getInfo
    pub fn get_info(&self) -> &PseudoActionInfo {
        &self.info
    }

    // Java: com/google/devtools/build/lib/analysis/PseudoAction.java:105 PseudoAction.getDummyOutput
    pub fn get_dummy_output(rule_context: &RuleContext) -> Artifact {
        rule_context.get_package_relative_artifact(
            &PathFragment::create(&format!(
                "{}.extra_action_dummy",
                rule_context.get_label().get_name()
            )),
            &rule_context.get_genfiles_directory(),
        )
    }
}

/// `UUID.fromString(s)`: (least significant bits, most significant bits) of a canonical UUID.
fn parse_uuid(s: &str) -> (i64, i64) {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    assert_eq!(
        hex.len(),
        32,
        "IllegalArgumentException: Invalid UUID string: {s}"
    );
    let most = u64::from_str_radix(&hex[..16], 16).expect("UUID") as i64;
    let least = u64::from_str_radix(&hex[16..], 16).expect("UUID") as i64;
    (least, most)
}

impl Action for PseudoAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/PseudoAction.java:82 PseudoAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        self.mnemonic.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/PseudoAction.java:87 PseudoAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_uuid(self.uuid.0, self.uuid.1);
        fp.add_bytes(&self.info.to_byte_array());
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/PseudoAction.java:100 PseudoAction.getExtraActionInfo
    fn get_extra_action_info(
        &self,
        action_key_context: &ActionKeyContext,
    ) -> Result<ExtraActionInfo, CommandLineError> {
        let mut builder = blaze_actions::actions::action::abstract_get_extra_action_info(
            self,
            action_key_context,
        )?;
        self.info.set_on(&mut builder);
        Ok(builder)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_uuid;

    /// `UUID.fromString("8d720129-bc1a-481f-8c4c-dbe11dcef319")`: most significant bits
    /// 0x8d720129bc1a481f, least 0x8c4cdbe11dcef319.
    #[test]
    fn parse_uuid_splits_most_and_least_significant_bits() {
        let (least, most) = parse_uuid("8d720129-bc1a-481f-8c4c-dbe11dcef319");
        assert_eq!(most as u64, 0x8d72_0129_bc1a_481f);
        assert_eq!(least as u64, 0x8c4c_dbe1_1dce_f319);
    }
}
