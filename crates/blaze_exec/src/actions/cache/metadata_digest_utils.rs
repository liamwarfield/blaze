//! Port of `lib/actions/cache/MetadataDigestUtils.java`, plus `FileArtifactValue.addTo` and
//! `getModifiedTime` (which the digest needs) over the seam [`FileArtifactValue`].

use std::collections::HashMap;
use std::io::{self, Write};

use blaze_util::Fingerprint;

use super::digest_utils::DigestUtils;
use super::var_int::{ByteBuffer, VarInt};
use crate::actions::file_artifact_value::FileArtifactValue;

/// Utility class for digests/metadata relating to the action cache.
// Java: com/google/devtools/build/lib/actions/cache/MetadataDigestUtils.java:28 MetadataDigestUtils
pub struct MetadataDigestUtils;

impl MetadataDigestUtils {
    /// Returns the digest read from the given buffer.
    // Java: com/google/devtools/build/lib/actions/cache/MetadataDigestUtils.java:35 MetadataDigestUtils.read
    pub fn read(source: &mut ByteBuffer<'_>) -> io::Result<Vec<u8>> {
        let size = VarInt::get_var_int(source)?;
        if size < 0 {
            return Err(io::Error::other(format!("Negative digest size: {}", size)));
        }
        let mut bytes = vec![0u8; size as usize];
        source.get_bytes(&mut bytes)?;
        Ok(bytes)
    }

    /// Write the digest to the output stream.
    // Java: com/google/devtools/build/lib/actions/cache/MetadataDigestUtils.java:46 MetadataDigestUtils.write
    pub fn write(digest: &[u8], sink: &mut dyn Write) -> io::Result<()> {
        VarInt::put_var_int(digest.len() as i32, sink)?;
        sink.write_all(digest)
    }

    /// Computes an order-independent digest from the given (path, metadata) pairs. Values may be
    /// `None`.
    // Java: com/google/devtools/build/lib/actions/cache/MetadataDigestUtils.java:56 MetadataDigestUtils.fromMetadata
    pub fn from_metadata(md_map: &HashMap<String, Option<FileArtifactValue>>) -> Vec<u8> {
        let mut result = vec![0u8; 1]; // reserve the empty string
        // Profiling showed that MessageDigest engine instantiation was a hotspot, so create one
        // instance for this computation to amortize its cost.
        let mut fp = Fingerprint::new();
        // The combination is order-independent, so HashMap iteration order does not matter.
        for (exec_path, md) in md_map {
            result = DigestUtils::combine_unordered(
                result,
                MetadataDigestUtils::get_digest(&mut fp, exec_path, md.as_ref()),
            );
        }
        result
    }

    // Java: com/google/devtools/build/lib/actions/cache/MetadataDigestUtils.java:68 MetadataDigestUtils.getDigest
    fn get_digest(
        fp: &mut Fingerprint,
        exec_path: &str,
        md: Option<&FileArtifactValue>,
    ) -> Vec<u8> {
        // Exec paths are internal strings in Java; `Fingerprint::add_string` hashes the internal
        // string a Rust string stands for (the `blaze_util::string_encoding` seam).
        fp.add_string(exec_path);
        if let Some(md) = md {
            add_to(md, fp);
        }
        fp.digest_and_reset()
    }
}

/// `FileArtifactValue.addTo(Fingerprint)`: adds this file metadata to the given fingerprint.
// Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:255 FileArtifactValue.addTo
pub fn add_to(md: &FileArtifactValue, fp: &mut Fingerprint) {
    match md.get_digest() {
        Some(digest) => {
            fp.add_bytes(digest);
        }
        None => {
            // Use the timestamp if the digest is not present, but not both. Modifying a timestamp
            // while keeping the contents of a file the same should not cause rebuilds.
            fp.add_long(get_modified_time(md));
        }
    }
}

/// `FileArtifactValue.getModifiedTime()` for each subclass.
// Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:113 FileArtifactValue.getModifiedTime
pub fn get_modified_time(md: &FileArtifactValue) -> i64 {
    match md {
        // Java: FileArtifactValue.java:503 DirectoryArtifactValue.getModifiedTime
        FileArtifactValue::Directory { mtime } => *mtime,
        // Java: FileArtifactValue.java:557 HashedDirectoryArtifactValue.getModifiedTime
        FileArtifactValue::HashedDirectory { .. } => 0,
        // Java: TreeArtifactValue.java:281 TreeArtifactCompositeFileArtifactValue.getModifiedTime
        FileArtifactValue::TreeArtifactComposite { .. } => {
            panic!("UnsupportedOperationException")
        }
        // Java: FileArtifactValue.java:667 RegularFileArtifactValue.getModifiedTime
        FileArtifactValue::RegularFile { .. } => panic!(
            "UnsupportedOperationException: regular file's mtime should never be called. ({})",
            file_artifact_value_to_string(md)
        ),
        // Java: FileArtifactValue.java:716 RunfilesProxyArtifactValue.getModifiedTime
        FileArtifactValue::RunfilesProxy { .. } => panic!(
            "UnsupportedOperationException: runfile proxy's mtime should never be called. ({})",
            file_artifact_value_to_string(md)
        ),
        // Java: FileArtifactValue.java:790 RemoteFileArtifactValue.getModifiedTime
        FileArtifactValue::RemoteFile { .. } => panic!(
            "UnsupportedOperationException: RemoteFileArtifactValue doesn't support getModifiedTime"
        ),
        // Java: FileArtifactValue.java:1012 ResolvedSymlinkArtifactValue.getModifiedTime
        FileArtifactValue::ResolvedSymlink { delegate, .. } => get_modified_time(delegate),
        // Java: FileArtifactValue.java:1191 UnresolvedSymlinkArtifactValue.getModifiedTime
        FileArtifactValue::UnresolvedSymlink { .. } => panic!("IllegalStateException"),
        // Java: FileArtifactValue.java:1372 ProxyFileArtifactValue.getModifiedTime
        FileArtifactValue::Proxy { delegate, .. } => get_modified_time(delegate),
        // Java: FileArtifactValue.java:1410 SingletonMarkerValue.getModifiedTime
        FileArtifactValue::RunfilesTreeMarker | FileArtifactValue::MissingFileMarker => 0,
        // Java: FileArtifactValue.java:1456 ConstantMetadataValue.getModifiedTime
        FileArtifactValue::ConstantMetadata => -1,
    }
}

/// `FileArtifactValue.bytesToString`.
// Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:458 FileArtifactValue.bytesToString
fn bytes_to_string(bytes: Option<&[u8]>) -> String {
    match bytes {
        None => "null".to_string(),
        Some(bytes) => format!("0x{}", hex::encode_upper(bytes)),
    }
}

/// `FileArtifactValue.getExpirationTime()` as epoch millis (`None`: never expires).
// Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:210 FileArtifactValue.getExpirationTime
pub fn get_expiration_time(md: &FileArtifactValue) -> Option<i64> {
    match md {
        FileArtifactValue::RemoteFile {
            expiration_time_millis,
            ..
        } => expiration_time_millis.filter(|millis| *millis >= 0),
        FileArtifactValue::ResolvedSymlink { delegate, .. } => get_expiration_time(delegate),
        _ => None,
    }
}

/// `FileArtifactValue.isInMemoryOutput()`.
// Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:156 FileArtifactValue.isInMemoryOutput
pub fn is_in_memory_output(md: &FileArtifactValue) -> bool {
    match md {
        FileArtifactValue::RemoteFile {
            in_memory_output, ..
        } => *in_memory_output,
        FileArtifactValue::ResolvedSymlink { delegate, .. } => is_in_memory_output(delegate),
        FileArtifactValue::Proxy { delegate, .. } => is_in_memory_output(delegate),
        _ => false,
    }
}

/// `metadata instanceof ProxyFileArtifactValue`.
pub fn is_proxy(md: &FileArtifactValue) -> bool {
    matches!(md, FileArtifactValue::Proxy { .. })
}

/// Java's `FileArtifactValue.toString()` for each subclass (used by `ActionCache.Entry`'s
/// `toString` and `dump`).
pub fn file_artifact_value_to_string(md: &FileArtifactValue) -> String {
    match md {
        // Java: FileArtifactValue.java:518 DirectoryArtifactValue.toString
        FileArtifactValue::Directory { mtime } => {
            format!("DirectoryArtifactValue{{mtime={}}}", mtime)
        }
        // Java: FileArtifactValue.java:575 HashedDirectoryArtifactValue.toString
        FileArtifactValue::HashedDirectory { digest } => format!(
            "HashedDirectoryArtifactValue{{digest={}}}",
            bytes_to_string(Some(digest))
        ),
        // Java: TreeArtifactValue.java:285 TreeArtifactCompositeFileArtifactValue.toString
        FileArtifactValue::TreeArtifactComposite {
            digest,
            resolved_path,
            ..
        } => format!(
            "TreeArtifactCompositeFileArtifactValue{{digest={}, resolvedPath={}}}",
            digest
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            resolved_path
                .as_ref()
                .map_or("null".to_string(), |p| p.get_path_string().to_string())
        ),
        // Java: FileArtifactValue.java:673 RegularFileArtifactValue.toString
        FileArtifactValue::RegularFile {
            digest,
            proxy,
            size,
        } => format!(
            "RegularFileArtifactValue{{digest={}, size={}, proxy={}}}",
            bytes_to_string(digest.as_deref()),
            size,
            match proxy {
                // Java: FileContentsProxy.toString is prettyPrint.
                Some(p) => p.pretty_print(),
                None => "null".to_string(),
            }
        ),
        // Java: FileArtifactValue.java:738 RunfilesProxyArtifactValue.toString
        FileArtifactValue::RunfilesProxy { digest } => format!(
            "RunfilesProxyArtifactValue{{digest={}}}",
            bytes_to_string(Some(digest))
        ),
        FileArtifactValue::RemoteFile {
            digest,
            size,
            location_index,
            expiration_time_millis,
            in_memory_output,
        } => {
            // PORT: the seam enum merges RemoteFileArtifactValue and
            // RemoteFileArtifactValueWithMaterializationData; the latter is the one that carries
            // an expiration time or the in-memory flag (as decodeRemoteMetadata creates them).
            if expiration_time_millis.is_some() || *in_memory_output {
                // Java: FileArtifactValue.java:922 RemoteFileArtifactValueWithMaterializationData.toString
                format!(
                    "RemoteFileArtifactValueWithMaterializationData{{digest={}, size={}, locationIndex={}, expirationTime={}, proxy=null, materializedAsToplevelOutput=false, inMemoryOutput={}}}",
                    bytes_to_string(Some(digest)),
                    size,
                    location_index,
                    match get_expiration_time(md) {
                        Some(millis) => instant_to_string(millis),
                        None => "null".to_string(),
                    },
                    in_memory_output
                )
            } else {
                // Java: FileArtifactValue.java:811 RemoteFileArtifactValue.toString
                format!(
                    "RemoteFileArtifactValue{{digest={}, size={}, locationIndex={}}}",
                    bytes_to_string(Some(digest)),
                    size,
                    location_index
                )
            }
        }
        // Java: FileArtifactValue.java:1079 ResolvedSymlinkArtifactValue.toString
        FileArtifactValue::ResolvedSymlink {
            delegate,
            resolved_path,
        } => format!(
            "ResolvedSymlinkArtifactValue{{delegate={}, resolvedPath={}}}",
            file_artifact_value_to_string(delegate),
            resolved_path
        ),
        // Java: FileArtifactValue.java:1145 UnresolvedSymlinkArtifactValue inherits
        // Object.toString (class name and identity hash code).
        FileArtifactValue::UnresolvedSymlink { .. } => "UnresolvedSymlinkArtifactValue".to_string(),
        // Java: FileArtifactValue.java:1382 ProxyFileArtifactValue.toString
        FileArtifactValue::Proxy { delegate, path } => format!(
            "ProxyFileArtifactValue{{delegate={}, path={}}}",
            file_artifact_value_to_string(delegate),
            path
        ),
        // Java: FileArtifactValue.java:1425 SingletonMarkerValue.toString (identity hash code)
        FileArtifactValue::RunfilesTreeMarker | FileArtifactValue::MissingFileMarker => {
            "singleton marker artifact value".to_string()
        }
        // Java: ConstantMetadataValue inherits Object.toString.
        FileArtifactValue::ConstantMetadata => "ConstantMetadataValue".to_string(),
    }
}

/// `java.time.Instant.toString()` (ISO-8601, UTC) for epoch milliseconds.
pub fn instant_to_string(epoch_millis: i64) -> String {
    let secs = epoch_millis.div_euclid(1000);
    let millis = epoch_millis.rem_euclid(1000);
    let days = secs.div_euclid(86400);
    let sod = secs.rem_euclid(86400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let year = if y > 9999 {
        format!("+{}", y)
    } else if y < 0 {
        format!("-{:04}", -y)
    } else {
        format!("{:04}", y)
    };
    let mut s = format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}",
        year,
        m,
        d,
        sod / 3600,
        (sod / 60) % 60,
        sod % 60
    );
    if millis != 0 {
        s.push_str(&format!(".{:03}", millis));
    }
    s.push('Z');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instant_to_string_matches_java() {
        assert_eq!(instant_to_string(0), "1970-01-01T00:00:00Z");
        assert_eq!(instant_to_string(86_400_000 * 600), "1971-08-24T00:00:00Z");
        assert_eq!(instant_to_string(1_500), "1970-01-01T00:00:01.500Z");
    }

    #[test]
    fn digest_round_trip() {
        let mut out = Vec::new();
        MetadataDigestUtils::write(&[1, 2, 3], &mut out).unwrap();
        let mut buf = ByteBuffer::wrap(&out);
        assert_eq!(MetadataDigestUtils::read(&mut buf).unwrap(), vec![1, 2, 3]);
    }
}
