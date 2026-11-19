//! Port of `GetCredentialsResponse`.

use indexmap::IndexMap;

use super::gson::{self, JsonReader, JsonSyntaxException, JsonToken, JsonWriter};
use super::java_time::{self, Instant};

/// Response from the `get` command of the
/// [Credential Helper Protocol](https://github.com/bazelbuild/proposals/blob/main/designs/2022-06-07-bazel-credential-helpers.md#proposal).
///
/// See the [specification](https://github.com/EngFlow/credential-helper-spec/blob/main/schemas/get-credentials-response.schema.json).
///
/// Equality is the record's: the headers compare as maps (order-insensitive, as
/// `ImmutableMap.equals`).
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsResponse.java:55 GetCredentialsResponse
#[derive(Clone, PartialEq, Eq)]
pub struct GetCredentialsResponse {
    headers: IndexMap<String, Vec<String>>,
    expires: Option<Instant>,
}

impl std::fmt::Debug for GetCredentialsResponse {
    /// Prints the header names only: the values are credentials.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GetCredentialsResponse")
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .field("expires", &self.expires)
            .finish()
    }
}

impl GetCredentialsResponse {
    /// Returns the headers to attach to the request.
    pub fn headers(&self) -> &IndexMap<String, Vec<String>> {
        &self.headers
    }

    /// Returns the time the credentials expire and must be revalidated.
    pub fn expires(&self) -> Option<Instant> {
        self.expires
    }

    /// Returns a new builder for [`GetCredentialsResponse`].
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsResponse.java:68 GetCredentialsResponse.newBuilder
    pub fn new_builder() -> GetCredentialsResponseBuilder {
        GetCredentialsResponseBuilder::default()
    }

    /// `GSON.toJson(response)`.
    pub fn to_json(&self) -> String {
        let mut writer = JsonWriter::new();
        GetCredentialsResponseGsonTypeAdapter::write(&mut writer, self);
        writer.into_string()
    }

    /// `GSON.fromJson(json, GetCredentialsResponse.class)`: `Ok(None)` for an empty document or
    /// `null`.
    pub fn from_json(json: &str) -> Result<Option<GetCredentialsResponse>, JsonSyntaxException> {
        gson::from_json(json, GetCredentialsResponseGsonTypeAdapter::read)
    }
}

/// `GetCredentialsResponse.RFC_3339_FORMATTER.format(instant)`.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsResponse.java:62 GetCredentialsResponse.RFC_3339_FORMATTER
pub fn rfc_3339_format(instant: &Instant) -> String {
    java_time::rfc_3339_format(instant)
}

/// Builder for [`GetCredentialsResponse`].
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsResponse.java:74 GetCredentialsResponse.Builder
#[derive(Clone, Default)]
pub struct GetCredentialsResponseBuilder {
    /// `headersBuilder()`: an `ImmutableMap.Builder`, which keeps every `put` until `build`.
    headers: Vec<(String, Vec<String>)>,
    expires: Option<Instant>,
}

impl GetCredentialsResponseBuilder {
    /// `headersBuilder().put(name, values)`.
    pub fn headers_builder_put(
        &mut self,
        name: impl Into<String>,
        values: Vec<String>,
    ) -> &mut Self {
        self.headers.push((name.into(), values));
        self
    }

    pub fn set_expires(&mut self, instant: Instant) -> &mut Self {
        self.expires = Some(instant);
        self
    }

    /// Returns the newly constructed [`GetCredentialsResponse`].
    ///
    /// `ImmutableMap.Builder.build()` (`buildOrThrow`) throws an `IllegalArgumentException` for a
    /// header name given twice; Gson does not catch it, so it is a panic.
    pub fn build(&self) -> GetCredentialsResponse {
        let mut headers: IndexMap<String, Vec<String>> = IndexMap::new();
        for (name, values) in &self.headers {
            if let Some(existing) = headers.get(name) {
                panic!(
                    "IllegalArgumentException: Multiple entries with same key: {name}=[{}] and {name}=[{}]",
                    existing.join(", "),
                    values.join(", ")
                );
            }
            headers.insert(name.clone(), values.clone());
        }
        GetCredentialsResponse {
            headers,
            expires: self.expires,
        }
    }
}

/// GSON adapter for GetCredentialsResponse.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsResponse.java:86 GetCredentialsResponse.GsonTypeAdapter
pub struct GetCredentialsResponseGsonTypeAdapter;

impl GetCredentialsResponseGsonTypeAdapter {
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsResponse.java:88 GetCredentialsResponse.GsonTypeAdapter.write
    pub fn write(writer: &mut JsonWriter, response: &GetCredentialsResponse) {
        writer.begin_object();

        let headers = response.headers();
        if !headers.is_empty() {
            writer.name("headers");
            writer.begin_object();
            for (name, values) in headers {
                writer.name(name);

                writer.begin_array();
                for value in values {
                    writer.value(value);
                }
                writer.end_array();
            }
            writer.end_object();
        }

        if let Some(expires) = response.expires() {
            writer.name("expires");
            writer.value(&rfc_3339_format(&expires));
        }

        writer.end_object();
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsResponse.java:121 GetCredentialsResponse.GsonTypeAdapter.read
    pub fn read(
        reader: &mut JsonReader<'_>,
    ) -> Result<GetCredentialsResponse, JsonSyntaxException> {
        let mut response = GetCredentialsResponse::new_builder();

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
                "headers" => {
                    if reader.peek()? != JsonToken::BeginObject {
                        return Err(JsonSyntaxException::new(format!(
                            "Expected value of 'headers' to be an object, got {}",
                            reader.peek()?
                        )));
                    }
                    reader.begin_object()?;

                    while reader.has_next()? {
                        let header_name = reader.next_name()?;
                        let mut header_values: Vec<String> = Vec::new();

                        if reader.peek()? != JsonToken::BeginArray {
                            return Err(JsonSyntaxException::new(format!(
                                "Expected value of '{}' header to be an array of strings, got {}",
                                header_name,
                                reader.peek()?
                            )));
                        }
                        reader.begin_array()?;
                        let mut i = 0;
                        while reader.has_next()? {
                            if reader.peek()? != JsonToken::String {
                                return Err(JsonSyntaxException::new(format!(
                                    "Expected value {} of '{}' header to be a string, got {}",
                                    i,
                                    header_name,
                                    reader.peek()?
                                )));
                            }
                            header_values.push(reader.next_string()?);
                            i += 1;
                        }
                        reader.end_array()?;

                        response.headers_builder_put(header_name, header_values);
                    }

                    reader.end_object()?;
                }
                "expires" => {
                    if reader.peek()? != JsonToken::String {
                        return Err(JsonSyntaxException::new(format!(
                            "Expected value of 'expires' to be a string, got {}",
                            reader.peek()?
                        )));
                    }
                    match java_time::rfc_3339_parse(&reader.next_string()?) {
                        Ok(instant) => {
                            response.set_expires(instant);
                        }
                        Err(e) => {
                            return Err(JsonSyntaxException::new(format!(
                                "Expected value of 'expires' to be a RFC 3339 formatted timestamp: {}",
                                e.message
                            )));
                        }
                    }
                }
                // We intentionally ignore unknown keys to achieve forward compatibility with
                // responses coming from newer tools.
                _ => reader.skip_value()?,
            }
        }
        reader.end_object()?;
        Ok(response.build())
    }
}
