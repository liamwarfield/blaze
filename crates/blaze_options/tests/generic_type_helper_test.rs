//! Port of src/test/java/com/google/devtools/common/options/GenericTypeHelperTest.java.
//! `GenericTypeHelper` resolves Java generic types by reflection; the Rust port has no
//! equivalent (converter result types are checked by the compiler), so every test is ignored.

const REASON: &str = "needs Java reflection (GenericTypeHelper has no Rust equivalent)";

macro_rules! reflection_test {
    ($($(#[$m:meta])* fn $name:ident;)*) => {$(
        $(#[$m])*
        #[test]
        #[ignore = "needs Java reflection (GenericTypeHelper has no Rust equivalent)"]
        fn $name() {
            blaze_options::unported!(REASON);
        }
    )*};
}

reflection_test! {
    /// Java: GenericTypeHelperTest.getConverterType
    fn get_converter_type;
    /// Java: GenericTypeHelperTest.getConverterTypeForGenericExtension
    fn get_converter_type_for_generic_extension;
    /// Java: GenericTypeHelperTest.getConverterTypeForGenericExtensionSecondGrade
    fn get_converter_type_for_generic_extension_second_grade;
    /// Java: GenericTypeHelperTest.getConverterTypeForParameterizedType
    fn get_converter_type_for_parameterized_type;
    /// Java: GenericTypeHelperTest.assignableFromPrimitive
    fn assignable_from_primitive;
    /// Java: GenericTypeHelperTest.assignableFromSuper
    fn assignable_from_super;
    /// Java: GenericTypeHelperTest.assignableFromSuperSecondGrade
    fn assignable_from_super_second_grade;
}
