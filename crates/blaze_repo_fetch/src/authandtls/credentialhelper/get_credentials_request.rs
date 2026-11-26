//! Request for the `get` command of the Credential Helper Protocol.

use super::gson::{self, JsonSyntaxException, JsonValue};
use crate::uri::JavaUri;

/// Request for the `get` command of the Credential Helper Protocol
/// (<https://github.com/bazelbuild/proposals/blob/main/designs/2022-06-07-bazel-credential-helpers.md#proposal>).
///
/// `uri`: the URI this request is for.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsRequest.java:39 GetCredentialsRequest
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GetCredentialsRequest {
    uri: JavaUri,
}

impl GetCredentialsRequest {
    /// `GetCredentialsRequest.newBuilder().setUri(uri).build()`.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsRequest.java:45 GetCredentialsRequest.newBuilder
    pub fn new(uri: JavaUri) -> GetCredentialsRequest {
        GetCredentialsRequest { uri }
    }

    /// `uri()`.
    pub fn uri(&self) -> &JavaUri {
        &self.uri
    }

    /// `GSON.toJson(request)` (`GsonTypeAdapter.write`).
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsRequest.java:62 GetCredentialsRequest.GsonTypeAdapter.write
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        out.push('{');
        gson::write_string(&mut out, "uri");
        out.push(':');
        gson::write_string(&mut out, self.uri.as_str());
        out.push('}');
        out
    }

    /// `GSON.fromJson(json, GetCredentialsRequest.class)`: `Ok(None)` is Java's `null`.
    // Java: com/google/gson/Gson.java Gson.fromJson
    pub fn parse(json: &str) -> Result<Option<GetCredentialsRequest>, JsonSyntaxException> {
        match gson::from_json(json)? {
            None => Ok(None),
            Some(value) => GetCredentialsRequest::from_json(&value).map(Some),
        }
    }

    /// `GsonTypeAdapter.read`, over the parsed document.
    ///
    /// Panics as `URI.create` does (`IllegalArgumentException`, which Gson does not wrap) on a
    /// `uri` that is not a URI.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsRequest.java:73 GetCredentialsRequest.GsonTypeAdapter.read
    // Kept as Java's `switch`.
    #[allow(clippy::single_match)]
    fn from_json(value: &JsonValue) -> Result<GetCredentialsRequest, JsonSyntaxException> {
        let mut uri = None;

        let JsonValue::Object(members) = value else {
            return Err(JsonSyntaxException(format!(
                "Expected object, got {}",
                value.token_name()
            )));
        };
        for (name, value) in members {
            match name.as_str() {
                "uri" => {
                    let JsonValue::String(s) = value else {
                        return Err(JsonSyntaxException(format!(
                            "Expected value of 'url' to be a string, got {}",
                            value.token_name()
                        )));
                    };
                    uri = Some(JavaUri::create(s).unwrap_or_else(|e| {
                        panic!("java.lang.IllegalArgumentException: {}", e.message)
                    }));
                }
                // We intentionally ignore unknown keys to achieve forward compatibility with
                // requests coming from newer tools.
                _ => {}
            }
        }
        match uri {
            Some(uri) => Ok(GetCredentialsRequest { uri }),
            // AutoBuilder's build() throws IllegalStateException, which Gson wraps:
            // `new JsonSyntaxException(e)`.
            None => Err(JsonSyntaxException(
                "java.lang.IllegalStateException: Missing required properties: uri".to_string(),
            )),
        }
    }
}
