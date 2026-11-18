//! Port of `packages/ProtoUtils.java`.
//!
//! PORT: move to blaze_packages (it depends on blaze_protos' `build.proto` types, which
//! blaze_packages does not depend on yet).

use blaze_packages::types::Type;
use blaze_protos::blaze_query::attribute::Discriminator;

/// Returns the `Discriminator` value corresponding to the provided `Type`.
// Java: com/google/devtools/build/lib/packages/ProtoUtils.java:82 ProtoUtils.getDiscriminatorFromType
pub fn get_discriminator_from_type(type_: Type) -> Discriminator {
    // Java: com/google/devtools/build/lib/packages/ProtoUtils.java:52 ProtoUtils.TYPE_MAP
    match type_ {
        Type::Integer => Discriminator::Integer,
        Type::Label => Discriminator::Label,
        Type::NodepLabel => Discriminator::String,
        Type::LabelList => Discriminator::LabelList,
        Type::GenqueryScopeType => Discriminator::Label,
        Type::GenqueryScopeTypeList => Discriminator::LabelList,
        Type::NodepLabelList => Discriminator::StringList,
        Type::DormantLabel => Discriminator::LabelList,
        Type::DormantLabelList => Discriminator::StringList,
        Type::String => Discriminator::String,
        Type::StringNoIntern => Discriminator::String,
        Type::StringList => Discriminator::StringList,
        Type::StringSet => Discriminator::DistributionSet,
        Type::Output => Discriminator::Output,
        Type::OutputList => Discriminator::OutputList,
        Type::License => Discriminator::License,
        Type::StringDict => Discriminator::StringDict,
        Type::LabelDictUnary => Discriminator::LabelDictUnary,
        Type::StringListDict => Discriminator::StringListDict,
        Type::LabelListDict => Discriminator::LabelListDict,
        Type::Boolean => Discriminator::Boolean,
        Type::Tristate => Discriminator::Tristate,
        Type::IntegerList => Discriminator::IntegerList,
        Type::LabelKeyedStringDict => Discriminator::LabelKeyedStringDict,
    }
}
