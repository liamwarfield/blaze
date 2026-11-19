//! Port of `GetCredentialsRequest`.

use super::gson::{self, JsonReader, JsonSyntaxException, JsonToken, JsonWriter};
use crate::uri::JavaUri;

/// Request for the `get` command of the
/// [Credential Helper Protocol](https://github.com/bazelbuild/proposals/blob/main/designs/2022-06-07-bazel-credential-helpers.md#proposal).
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsRequest.java:43 GetCredentialsRequest
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GetCredentialsRequest {
    uri: JavaUri,
}

impl GetCredentialsRequest {
    /// Returns the URI this request is for.
    pub fn uri(&self) -> &JavaUri {
        &self.uri
    }

    /// Returns a new builder for [`GetCredentialsRequest`].
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsRequest.java:49 GetCredentialsRequest.newBuilder
    pub fn new_builder() -> GetCredentialsRequestBuilder {
        GetCredentialsRequestBuilder { uri: None }
    }

    /// `GSON.toJson(request)`.
    pub fn to_json(&self) -> String {
        let mut writer = JsonWriter::new();
        GetCredentialsRequestGsonTypeAdapter::write(&mut writer, self);
        writer.into_string()
    }

    /// `GSON.fromJson(json, GetCredentialsRequest.class)`: `Ok(None)` for an empty document or
    /// `null`.
    pub fn from_json(json: &str) -> Result<Option<GetCredentialsRequest>, JsonSyntaxException> {
        gson::from_json(json, GetCredentialsRequestGsonTypeAdapter::read)
    }
}

/// Builder for [`GetCredentialsRequest`].
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsRequest.java:55 GetCredentialsRequest.Builder
#[derive(Clone, Debug, Default)]
pub struct GetCredentialsRequestBuilder {
    uri: Option<JavaUri>,
}

impl GetCredentialsRequestBuilder {
    /// Sets the URI this request is for.
    pub fn set_uri(mut self, uri: JavaUri) -> Self {
        self.uri = Some(uri);
        self
    }

    /// Returns the newly constructed [`GetCredentialsRequest`].
    ///
    /// PORT: AutoBuilder's `IllegalStateException("Missing required properties: uri")` is an
    /// `Err`, so that `Gson.fromJson` can turn it into a [`JsonSyntaxException`] as Java does.
    pub fn build(self) -> Result<GetCredentialsRequest, String> {
        match self.uri {
            Some(uri) => Ok(GetCredentialsRequest { uri }),
            None => Err("Missing required properties: uri".to_string()),
        }
    }
}

/// GSON adapter for GetCredentialsRequest.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsRequest.java:66 GetCredentialsRequest.GsonTypeAdapter
pub struct GetCredentialsRequestGsonTypeAdapter;

impl GetCredentialsRequestGsonTypeAdapter {
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsRequest.java:68 GetCredentialsRequest.GsonTypeAdapter.write
    pub fn write(writer: &mut JsonWriter, value: &GetCredentialsRequest) {
        writer.begin_object();
        writer.name("uri").value(value.uri().as_str());
        writer.end_object();
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsRequest.java:79 GetCredentialsRequest.GsonTypeAdapter.read
    pub fn read(reader: &mut JsonReader<'_>) -> Result<GetCredentialsRequest, JsonSyntaxException> {
        let mut request = GetCredentialsRequest::new_builder();

        if reader.peek()? != JsonToken::BeginObject {
            return Err(JsonSyntaxException::new(format!(
                "Expected object, got {}",
                reader.peek()?
            )));
        }
        reader.begin_object()?;
        while reader.has_next()? {
            let name = reader.next_name()?;
            match name.as_str() {
                "uri" => {
                    if reader.peek()? != JsonToken::String {
                        return Err(JsonSyntaxException::new(format!(
                            "Expected value of 'url' to be a string, got {}",
                            reader.peek()?
                        )));
                    }
                    let uri = reader.next_string()?;
                    // URI.create throws an IllegalArgumentException, which Gson does not catch.
                    request = request.set_uri(
                        JavaUri::create(&uri)
                            .unwrap_or_else(|e| panic!("IllegalArgumentException: {e}")),
                    );
                }
                // We intentionally ignore unknown keys to achieve forward compatibility with
                // requests coming from newer tools.
                _ => reader.skip_value()?,
            }
        }
        reader.end_object()?;
        request
            .build()
            .map_err(|e| JsonSyntaxException::new(format!("java.lang.IllegalStateException: {e}")))
    }
}
