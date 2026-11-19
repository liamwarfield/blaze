//! `BlazeCommandUtils.expandHelpTopic` / `getUsage` and `ResourceFileLoader`.
//!
//! `getOptions`, `getCommonOptions` are in [`crate::command_registry`] and `getStartupOptions` is
//! [`crate::runtime::BlazeRuntime::get_startup_options`].

use std::sync::Arc;

use blaze_options::definition::OptionsClassRef;
use blaze_options::{HelpVerbosity, OptionsParser};

use crate::command::CommandAnnotation;
use crate::module::BlazeModule;

include!(concat!(env!("OUT_DIR"), "/jar_resources.rs"));

/// `java.io.IOException` from [`ResourceFileLoader::load_resource`].
#[derive(Debug)]
pub struct ResourceIOException(pub String);

/// A little utility to load resources from the server jar.
///
/// PORT: Java loads them from the jar on the class path; Blaze embeds the resources it uses at
/// build time from the oracle's `A-server.jar` (`build.rs`, `JAR_RESOURCES`). A Java class is
/// given by its package path (`com/google/devtools/build/lib/runtime/commands`).
// Java: com/google/devtools/build/lib/util/ResourceFileLoader.java:29 ResourceFileLoader
pub struct ResourceFileLoader;

impl ResourceFileLoader {
    fn get_resource_bytes(resource: &str) -> Option<&'static [u8]> {
        JAR_RESOURCES
            .iter()
            .find(|(name, _)| *name == resource)
            .map(|(_, data)| *data)
    }

    // Java: com/google/devtools/build/lib/util/ResourceFileLoader.java:33 ResourceFileLoader.resourceExists
    pub fn resource_exists(relative_to_package: &str, resource_name: &str) -> bool {
        Self::get_resource_bytes(&Self::resolve_resource(relative_to_package, resource_name))
            .is_some()
    }

    /// Loads a text resource that is located in the directory of the package
    /// `relative_to_package`, using UTF8 encoding.
    // Java: com/google/devtools/build/lib/util/ResourceFileLoader.java:47 ResourceFileLoader.loadResource
    pub fn load_resource(
        relative_to_package: &str,
        resource_name: &str,
    ) -> Result<String, ResourceIOException> {
        let resource = Self::resolve_resource(relative_to_package, resource_name);
        match Self::get_resource_bytes(&resource) {
            None => Err(ResourceIOException(format!("{} not found.", resource_name))),
            // new String(bytes, UTF_8): malformed input is replaced.
            Some(bytes) => Ok(String::from_utf8_lossy(bytes).into_owned()),
        }
    }

    /// The bytes of an absolute resource (`Class.getResourceAsStream("/path")`).
    pub fn load_absolute_resource_bytes(resource: &str) -> Option<&'static [u8]> {
        Self::get_resource_bytes(resource.strip_prefix('/').unwrap_or(resource))
    }

    /// Converts a relative resource name and Java package to a full resource path, using the
    /// same logic as [`Self::load_resource`].
    // Java: com/google/devtools/build/lib/util/ResourceFileLoader.java:69 ResourceFileLoader.resolveResource
    pub fn resolve_resource(relative_to_package: &str, resource_name: &str) -> String {
        format!("{}/{}", relative_to_package, resource_name)
    }
}

/// The package (as a resource directory) of the Java class of a command, which is where its
/// `resource:` help text lives.
///
/// PORT: Java passes the command's class to `ResourceFileLoader`; Blaze's command annotations do
/// not carry the Java class, so the package of each command class of Bazel 9.3.2 is listed here.
pub fn command_resource_package(command_name: &str) -> &'static str {
    match command_name {
        // com.google.devtools.build.lib.bazel.commands.{FetchCommand,ModCommand,VendorCommand}
        "fetch" | "mod" | "vendor" => "com/google/devtools/build/lib/bazel/commands",
        // com.google.devtools.build.lib.runtime.mobileinstall.MobileInstallCommand
        "mobile-install" => "com/google/devtools/build/lib/runtime/mobileinstall",
        // com.google.devtools.build.lib.runtime.commands.*Command
        _ => "com/google/devtools/build/lib/runtime/commands",
    }
}

/// Utility class for functionality related to Blaze commands.
// Java: com/google/devtools/build/lib/runtime/BlazeCommandUtils.java:31 BlazeCommandUtils
pub struct BlazeCommandUtils;

impl BlazeCommandUtils {
    /// Returns the expansion of the specified help topic.
    ///
    /// `topic`: the name of the help topic; used in %{command} expansion. `help`: the text
    /// template of the help message. Certain %{x} variables will be expanded. A prefix of
    /// "resource:" means use the .jar resource of that name (relative to `resource_package`).
    /// `help_verbosity`: a tri-state verbosity option selecting between just names, names and
    /// syntax, and full description.
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandUtils.java:111 BlazeCommandUtils.expandHelpTopic
    pub fn expand_help_topic(
        topic: &str,
        help: &str,
        resource_package: &str,
        options: &[OptionsClassRef],
        help_verbosity: HelpVerbosity,
        product_name: &str,
    ) -> String {
        let parser = OptionsParser::builder()
            .options_classes(options)
            .build()
            .unwrap_or_else(|e| e.throw());

        let template = if let Some(resource_name) = help.strip_prefix("resource:") {
            match ResourceFileLoader::load_resource(resource_package, resource_name) {
                Ok(t) => t,
                Err(e) => panic!(
                    "IllegalStateException: failed to load help resource '{}' due to I/O error: {}",
                    resource_name, e.0
                ),
            }
        } else {
            help.to_string()
        };

        if !template.contains("%{options}") {
            panic!(
                "IllegalStateException: Help template for '{}' omits %{{options}}!",
                topic
            );
        }

        let option_str = parser
            .describe_options(help_verbosity)
            .replace("%{product}", product_name);

        let expanded = template
            .replace("%{product}", product_name)
            .replace("%{command}", topic)
            .replace("%{options}", &option_str);
        format!(
            "{}\n\n{}",
            java_trim(&expanded),
            if help_verbosity == HelpVerbosity::MEDIUM {
                "(Use 'help --long' for full details or --short to just enumerate options.)\n"
            } else {
                ""
            }
        )
    }

    /// The help page for this command.
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandUtils.java:156 BlazeCommandUtils.getUsage
    pub fn get_usage(
        command_annotation: &CommandAnnotation,
        verbosity: HelpVerbosity,
        options_suppliers: &[Arc<dyn BlazeModule>],
        product_name: &str,
    ) -> String {
        Self::expand_help_topic(
            command_annotation.name(),
            command_annotation.help,
            command_resource_package(command_annotation.name()),
            &crate::command_registry::get_options(command_annotation, options_suppliers),
            verbosity,
            product_name,
        )
    }
}

/// Java's `String.trim()`.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    // Java: src/test/java/com/google/devtools/build/lib/util/ResourceFileLoaderTest.java ResourceFileLoaderTest.loader
    #[test]
    #[ignore = "needs the test resource ResourceFileLoaderTest.message on the class path; Blaze \
                embeds only the server jar's resources"]
    fn loader() {
        let message = ResourceFileLoader::load_resource(
            "com/google/devtools/build/lib/util",
            "ResourceFileLoaderTest.message",
        )
        .unwrap();
        assert_eq!(message, "Hello, world.");
    }

    // Java: src/test/java/com/google/devtools/build/lib/util/ResourceFileLoaderTest.java ResourceFileLoaderTest.resourceNotFound
    #[test]
    fn resource_not_found() {
        let e = ResourceFileLoader::load_resource(
            "com/google/devtools/build/lib/util",
            "does_not_exist.txt",
        )
        .unwrap_err();
        assert_eq!(e.0, "does_not_exist.txt not found.");
    }

    /// Every `resource:` help text of a registered command is embedded and names `%{options}`.
    #[test]
    fn command_help_resources_are_embedded() {
        for name in [
            "build",
            "canonicalize-flags",
            "fetch",
            "mobile-install",
            "mod",
            "vendor",
            "version",
        ] {
            let file = match name {
                "canonicalize-flags" => "canonicalize.txt".to_string(),
                other => format!("{}.txt", other),
            };
            if name == "canonicalize-flags" {
                // CanonicalizeCommand's help is inline in its @Command annotation.
                assert!(ResourceFileLoader::resource_exists(
                    command_resource_package(name),
                    &file
                ));
                continue;
            }
            let text = ResourceFileLoader::load_resource(command_resource_package(name), &file)
                .unwrap_or_else(|e| panic!("{}: {}", name, e.0));
            assert!(text.contains("%{options}"), "{}", name);
        }
    }
}
