//! Port of `packages/BuildSetting.java`.

use std::fmt;

use crate::types::LabelClass;
use crate::types::Type;

/// Metadata of a Starlark build setting (`rule(build_setting = config.xxx())`) (Java
/// `BuildSetting`).
// Java: com/google/devtools/build/lib/packages/BuildSetting.java:26 BuildSetting
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildSetting {
    is_flag: bool,
    type_: Type,
    allow_multiple: bool,
    repeatable: bool,
}

impl BuildSetting {
    // Java: com/google/devtools/build/lib/packages/BuildSetting.java:39 BuildSetting.create
    pub fn create_full(is_flag: bool, type_: Type, allow_multiple: bool, repeatable: bool) -> Self {
        BuildSetting {
            is_flag,
            type_,
            allow_multiple,
            repeatable,
        }
    }

    // Java: com/google/devtools/build/lib/packages/BuildSetting.java:44 BuildSetting.create
    pub fn create(is_flag: bool, type_: Type) -> Self {
        blaze_util::check_state!(
            type_.get_label_class() != LabelClass::Dependency,
            "Build settings should not create a dependency with their default attribute"
        );
        BuildSetting::create_full(is_flag, type_, false, false)
    }

    // Java: com/google/devtools/build/lib/packages/BuildSetting.java:51 BuildSetting.getType
    pub fn get_type(&self) -> Type {
        self.type_
    }

    // Java: com/google/devtools/build/lib/packages/BuildSetting.java:56 BuildSetting.isFlag
    pub fn is_flag(&self) -> bool {
        self.is_flag
    }

    // Java: com/google/devtools/build/lib/packages/BuildSetting.java:60 BuildSetting.allowsMultiple
    pub fn allows_multiple(&self) -> bool {
        self.allow_multiple
    }

    // Java: com/google/devtools/build/lib/packages/BuildSetting.java:64 BuildSetting.isRepeatableFlag
    pub fn is_repeatable_flag(&self) -> bool {
        self.repeatable
    }
}

impl fmt::Display for BuildSetting {
    // Java: com/google/devtools/build/lib/packages/BuildSetting.java:69 BuildSetting.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<build_setting.{}>", self.type_)
    }
}
