//! The registry half of `bazel/bzlmod/VendorManager.java`: where a registry URL is vendored, and
//! reading and writing vendored registry files.
//!
//! PORT: `VendorManager.vendorRepos` (moving fetched repositories into the vendor directory)
//! needs repository fetching (`RepositoryUtils`, the repo contents cache), which this crate does
//! not see; it is `blaze_repo_rules::bzlmod::vendor_manager::vendor_repos`. Both work on the same
//! vendor directory.

use std::io;

use blaze_util::string_encoding::internal_to_platform;
use blaze_vfs::PathFragment;

use crate::checksum::Checksum;
use crate::java_uri::JavaUri;

// Java: com/google/devtools/build/lib/bazel/bzlmod/VendorManager.java:46 VendorManager.REGISTRIES_DIR
const REGISTRIES_DIR: &str = "_registries";

/// `VendorManager.EXTERNAL_ROOT_SYMLINK_NAME`: the symlink in the vendor directory to the
/// external repositories root.
// Java: com/google/devtools/build/lib/bazel/bzlmod/VendorManager.java:48 VendorManager.EXTERNAL_ROOT_SYMLINK_NAME
pub const EXTERNAL_ROOT_SYMLINK_NAME: &str = "bazel-external";

/// Class to manage the vendor directory.
// Java: com/google/devtools/build/lib/bazel/bzlmod/VendorManager.java:44 VendorManager
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VendorManager {
    vendor_directory: PathFragment,
}

/// `java.io.UnsupportedEncodingException` of `URLDecoder.decode` (never thrown for `UTF-8`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsupportedEncodingException(pub String);

impl VendorManager {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/VendorManager.java:53 VendorManager.<init>
    pub fn new(vendor_directory: PathFragment) -> Self {
        VendorManager { vendor_directory }
    }

    pub fn get_vendor_directory(&self) -> &PathFragment {
        &self.vendor_directory
    }

    /// Checks if the given URL is vendored.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/VendorManager.java:147 VendorManager.isUrlVendored
    pub fn is_url_vendored(&self, url: &JavaUri) -> Result<bool, UnsupportedEncodingException> {
        let path = self.get_vendor_path_for_url(url)?;
        // Path.isFile(): follows symlinks.
        Ok(
            std::fs::metadata(&*internal_to_platform(path.get_path_string()))
                .is_ok_and(|m| m.is_file()),
        )
    }

    /// Vendors the registry URL with the specified content.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/VendorManager.java:158 VendorManager.vendorRegistryUrl
    pub fn vendor_registry_url(&self, url: &JavaUri, content: &[u8]) -> io::Result<()> {
        let output_path = self
            .get_vendor_path_for_url(url)
            .map_err(|e| io::Error::other(e.0))?;
        let parent = output_path
            .get_parent_directory()
            .expect("NullPointerException: getParentDirectory()");
        std::fs::create_dir_all(&*internal_to_platform(parent.get_path_string()))?;
        std::fs::write(
            &*internal_to_platform(output_path.get_path_string()),
            content,
        )
    }

    /// Reads the content of the registry URL and verifies its checksum. An `Err` is the message
    /// of Java's `IOException`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/VendorManager.java:172 VendorManager.readRegistryUrl
    pub fn read_registry_url(&self, url: &JavaUri, checksum: &Checksum) -> Result<Vec<u8>, String> {
        let path = self.get_vendor_path_for_url(url).map_err(|e| e.0)?;
        let content = std::fs::read(&*internal_to_platform(path.get_path_string()))
            .map_err(|e| format!("{} ({})", path.get_path_string(), io_reason(&e)))?;
        // checksum.getKeyType().newHasher().putBytes(content).hash()
        let actual: Vec<u8> = crate::checksum::hash_with(checksum.get_key_type(), &content);
        if checksum.get_hash_code() != actual.as_slice() {
            return Err(format!(
                "Checksum was {} but wanted {}",
                checksum.emit_other_hash_in_same_format(&actual),
                checksum.emit_other_hash_in_same_format(checksum.get_hash_code())
            ));
        }
        Ok(content)
    }

    /// Returns the vendor path for the given URL:
    /// `<vendor_directory>/_registries/<host>/<path>`.
    ///
    /// The host name is case-insensitive, so it is converted to lowercase. The path is
    /// case-sensitive, so it is left as is. The port number is not included in the vendor path.
    ///
    /// Note that the vendor path may conflict if two URLs only differ by the case or port
    /// number. But this is unlikely to happen in practice, and conflicts are checked in
    /// `VendorCommand`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/VendorManager.java:226 VendorManager.getVendorPathForUrl
    pub fn get_vendor_path_for_url(
        &self,
        url: &JavaUri,
    ) -> Result<PathFragment, UnsupportedEncodingException> {
        // Host names are case-insensitive.
        let host = uri_host(url)
            .unwrap_or_else(|| panic!("NullPointerException: getHost() of {}", url))
            .to_ascii_lowercase();
        let path = url.get_path().unwrap_or_default();
        let mut path = url_decode_utf8(&path);
        if path.starts_with('/') {
            path = path[1..].to_string();
        }
        Ok(self
            .vendor_directory
            .get_relative_str(REGISTRIES_DIR)
            .get_relative_str(&host)
            .get_relative_str(&path))
    }
}

/// `IOException.getMessage()` of a failed `FileSystemUtils.readContent`: the file's
/// `FileNotFoundException` / errno text.
fn io_reason(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => "No such file or directory".to_string(),
        _ => e.to_string(),
    }
}

/// `URI.getHost()`: the host of a server-based authority (`[userinfo@]host[:port]`), `None` when
/// the authority is absent or not server-based.
// Java (JDK): java/net/URI.java URI.Parser.parseServer
pub fn uri_host(url: &JavaUri) -> Option<String> {
    let authority = url.get_raw_authority()?;
    let rest = match authority.find('@') {
        Some(i) => &authority[i + 1..],
        None => authority,
    };
    let host = if rest.starts_with('[') {
        let end = rest.find(']')?;
        let (host, after) = rest.split_at(end + 1);
        if !after.is_empty() && !after.starts_with(':') {
            return None;
        }
        host
    } else {
        match rest.rfind(':') {
            Some(i) => {
                if !rest[i + 1..].bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                &rest[..i]
            }
            None => rest,
        }
    };
    if host.is_empty() {
        return None;
    }
    if !host.starts_with('[')
        && !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        return None;
    }
    Some(host.to_string())
}

/// `URLDecoder.decode(s, "UTF-8")`: `+` is a space and each run of `%XX` escapes is decoded as
/// UTF-8 (malformed input becomes U+FFFD); a malformed escape is an
/// `IllegalArgumentException`.
// Java (JDK): java/net/URLDecoder.java URLDecoder.decode
fn url_decode_utf8(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '+' => {
                out.push(' ');
                i += 1;
            }
            '%' => {
                let mut bytes = Vec::new();
                while i < chars.len() && chars[i] == '%' {
                    let hex: String = chars.get(i + 1..i + 3).map(|c| c.iter().collect()).unwrap_or_else(|| {
                        panic!(
                            "IllegalArgumentException: URLDecoder: Incomplete trailing escape (%) pattern"
                        )
                    });
                    let byte = u8::from_str_radix(&hex, 16).unwrap_or_else(|_| {
                        panic!(
                            "IllegalArgumentException: URLDecoder: Illegal hex characters in escape (%) pattern"
                        )
                    });
                    bytes.push(byte);
                    i += 3;
                }
                out.push_str(&String::from_utf8_lossy(&bytes));
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_path_for_url_drops_the_port_and_lowercases_the_host() {
        let m = VendorManager::new(PathFragment::create("/ws/vendor"));
        let url = JavaUri::new("http://LocalHost:8080/modules/aaa/1.0/MODULE.bazel").unwrap();
        assert_eq!(
            m.get_vendor_path_for_url(&url).unwrap().get_path_string(),
            "/ws/vendor/_registries/localhost/modules/aaa/1.0/MODULE.bazel"
        );
    }

    #[test]
    fn url_decoder_decodes_plus_and_escapes() {
        assert_eq!(url_decode_utf8("/a+b%2Bc%C3%A9"), "/a b+c\u{e9}");
    }
}
