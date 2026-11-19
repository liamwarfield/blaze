//! Port of `lib/remote/RemoteGrpcServiceConfig.java`.

use std::io::{self, Read};
use std::time::Duration;

use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_vfs::Path;
use serde_json::{Map, Value, json};

use crate::grpc::methods;

const SUPPORTED_TOP_LEVEL_FIELDS: &[&str] = &["methodConfig"];
const SUPPORTED_METHOD_CONFIG_FIELDS: &[&str] = &["name", "timeout"];
const SUPPORTED_NAME_FIELDS: &[&str] = &["service", "method"];

/// `FetchGrpc.SERVICE_NAME` (the Remote Asset API; its stubs are not ported).
const FETCH_SERVICE_NAME: &str = "build.bazel.remote.asset.v1.Fetch";

/// Creates gRPC service config values for Bazel's remote APIs.
///
/// `create(options, workingDirectory)`: the config from `--remote_grpc_service_config`, or the
/// one generated from `--remote_timeout`.
// Java: com/google/devtools/build/lib/remote/RemoteGrpcServiceConfig.java:60 RemoteGrpcServiceConfig.create
pub fn create(options: &RemoteOptions, working_directory: &Path) -> io::Result<Map<String, Value>> {
    let Some(service_config_path) = &options.remote_grpc_service_config else {
        return Ok(create_from_timeout(options.remote_timeout));
    };
    parse(&working_directory.get_relative(service_config_path.get()))
}

/// `create(Duration remoteTimeout)`.
// Java: com/google/devtools/build/lib/remote/RemoteGrpcServiceConfig.java:69 RemoteGrpcServiceConfig.create
pub fn create_from_timeout(remote_timeout: Duration) -> Map<String, Value> {
    let config = json!({
        "methodConfig": [
            {
                "name": [
                    {"service": methods::GET_ACTION_RESULT.service_name()},
                    {"service": methods::GET_CAPABILITIES.service_name()},
                    {"service": methods::FIND_MISSING_BLOBS.service_name()},
                    {"service": methods::READ.service_name()},
                    {"service": FETCH_SERVICE_NAME},
                ],
                "timeout": format!("{}s", remote_timeout.as_secs()),
            }
        ]
    });
    match config {
        Value::Object(map) => map,
        _ => unreachable!(),
    }
}

// Java: com/google/devtools/build/lib/remote/RemoteGrpcServiceConfig.java:85 RemoteGrpcServiceConfig.parse
fn parse(service_config_path: &Path) -> io::Result<Map<String, Value>> {
    // `new InputStreamReader(..., UTF_8)`: malformed input becomes U+FFFD; the text is Unicode,
    // as Gson gets it.
    let mut bytes = Vec::new();
    service_config_path
        .get_input_stream()?
        .read_to_end(&mut bytes)?;
    let text = blaze_util::string_encoding::java_new_string_utf8(&bytes);
    let root: Value = serde_json::from_str(&text).map_err(|e| {
        // PORT: the text after the path is Gson's JsonParseException message in Java.
        io::Error::other(format!(
            "failed to parse {}: {}",
            service_config_path.get_path_string(),
            e
        ))
    })?;
    let root = require_object(&root, "service config")?;
    reject_unsupported_fields(root, SUPPORTED_TOP_LEVEL_FIELDS, "service config")?;
    reject_unsupported_method_config_fields(root)?;
    Ok(root.clone())
}

// Java: com/google/devtools/build/lib/remote/RemoteGrpcServiceConfig.java:98 RemoteGrpcServiceConfig.rejectUnsupportedMethodConfigFields
fn reject_unsupported_method_config_fields(root_object: &Map<String, Value>) -> io::Result<()> {
    let Some(Value::Array(method_configs)) = root_object.get("methodConfig") else {
        return Ok(());
    };
    for (i, method_config_element) in method_configs.iter().enumerate() {
        let Value::Object(method_config) = method_config_element else {
            continue;
        };
        let method_config_path = format!("methodConfig[{i}]");
        reject_unsupported_fields(
            method_config,
            SUPPORTED_METHOD_CONFIG_FIELDS,
            &method_config_path,
        )?;
        reject_unsupported_name_fields(method_config, &method_config_path)?;
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/remote/RemoteGrpcServiceConfig.java:118 RemoteGrpcServiceConfig.rejectUnsupportedNameFields
fn reject_unsupported_name_fields(
    method_config: &Map<String, Value>,
    method_config_path: &str,
) -> io::Result<()> {
    let Some(Value::Array(names)) = method_config.get("name") else {
        return Ok(());
    };
    for (i, name_element) in names.iter().enumerate() {
        if let Value::Object(name) = name_element {
            reject_unsupported_fields(
                name,
                SUPPORTED_NAME_FIELDS,
                &format!("{method_config_path}.name[{i}]"),
            )?;
        }
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/remote/RemoteGrpcServiceConfig.java:137 RemoteGrpcServiceConfig.rejectUnsupportedFields
fn reject_unsupported_fields(
    object: &Map<String, Value>,
    supported_fields: &[&str],
    path: &str,
) -> io::Result<()> {
    for key in object.keys() {
        if !supported_fields.contains(&key.as_str()) {
            return Err(io::Error::other(format!(
                "{path} contains unsupported field '{key}'"
            )));
        }
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/remote/RemoteGrpcServiceConfig.java:146 RemoteGrpcServiceConfig.requireObject
fn require_object<'a>(element: &'a Value, path: &str) -> io::Result<&'a Map<String, Value>> {
    match element {
        Value::Object(map) => Ok(map),
        _ => Err(io::Error::other(format!("{path} must be an object"))),
    }
}
