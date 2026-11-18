//! The default-label string constants of `rules/proto/ProtoConstants.java` that option defaults
//! use.

// The flags need to point to @bazel_tools, because this is a canonical repo
// name when either bzlmod or WORKSPACE mode is used.
// Java: com/google/devtools/build/lib/rules/proto/ProtoConstants.java:44 ProtoConstants.DEFAULT_PROTOC_LABEL
/// Default label for proto compiler.
pub const DEFAULT_PROTOC_LABEL: &str = "@bazel_tools//tools/proto:protoc";

// Java: com/google/devtools/build/lib/rules/proto/ProtoConstants.java:47 ProtoConstants.DEFAULT_JAVA_PROTO_LABEL
/// Default label for java proto toolchains.
pub const DEFAULT_JAVA_PROTO_LABEL: &str = "@bazel_tools//tools/proto:java_toolchain";

// Java: com/google/devtools/build/lib/rules/proto/ProtoConstants.java:50 ProtoConstants.DEFAULT_JAVA_LITE_PROTO_LABEL
/// Default label for java lite proto toolchains.
pub const DEFAULT_JAVA_LITE_PROTO_LABEL: &str = "@bazel_tools//tools/proto:javalite_toolchain";

// Java: com/google/devtools/build/lib/rules/proto/ProtoConstants.java:54 ProtoConstants.DEFAULT_CC_PROTO_LABEL
/// Default label for cc proto toolchains.
pub const DEFAULT_CC_PROTO_LABEL: &str = "@bazel_tools//tools/proto:cc_toolchain";

// Java: com/google/devtools/build/lib/rules/proto/ProtoConstants.java:57 ProtoConstants.DEFAULT_J2OBJC_PROTO_LABEL
/// Default label for j2objc proto toolchains.
pub const DEFAULT_J2OBJC_PROTO_LABEL: &str = "@bazel_tools//tools/j2objc:j2objc_proto_toolchain";
