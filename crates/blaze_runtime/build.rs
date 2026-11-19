//! Extracts the server jar's text resources that commands print (help texts, the help
//! topics, the bash completion scripts, the embedded license), which Java loads with
//! `ResourceFileLoader` / `Class.getResourceAsStream`, from the oracle's `A-server.jar` (the
//! install base `crates/blaze/build.rs` embeds): `$OUT_DIR/jar_resources/<path>` and
//! `$OUT_DIR/jar_resources.rs` (`JAR_RESOURCES`: resource path to bytes).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

/// The jar resources that Blaze's commands load, by their path in the jar.
const JAR_RESOURCES: &[&str] = &[
    "com/google/devtools/build/lib/bazel/commands/fetch.txt",
    "com/google/devtools/build/lib/bazel/commands/mod.txt",
    "com/google/devtools/build/lib/bazel/commands/vendor.txt",
    "com/google/devtools/build/lib/runtime/commands/aquery.txt",
    "com/google/devtools/build/lib/runtime/commands/build.txt",
    "com/google/devtools/build/lib/runtime/commands/canonicalize.txt",
    "com/google/devtools/build/lib/runtime/commands/clean.txt",
    "com/google/devtools/build/lib/runtime/commands/config.txt",
    "com/google/devtools/build/lib/runtime/commands/coverage.txt",
    "com/google/devtools/build/lib/runtime/commands/cquery.txt",
    "com/google/devtools/build/lib/runtime/commands/help.txt",
    "com/google/devtools/build/lib/runtime/commands/info.txt",
    "com/google/devtools/build/lib/runtime/commands/license/LICENSE",
    "com/google/devtools/build/lib/runtime/commands/print_action.txt",
    "com/google/devtools/build/lib/runtime/commands/query.txt",
    "com/google/devtools/build/lib/runtime/commands/run.txt",
    "com/google/devtools/build/lib/runtime/commands/startup_options.txt",
    "com/google/devtools/build/lib/runtime/commands/target-syntax.txt",
    "com/google/devtools/build/lib/runtime/commands/test.txt",
    "com/google/devtools/build/lib/runtime/commands/version.txt",
    "com/google/devtools/build/lib/runtime/mobileinstall/mobile-install.txt",
    "scripts/bazel-complete-header.bash",
    "scripts/bazel-complete-template.bash",
];

/// The oracle's install base (as in `crates/blaze/build.rs`): `$BLAZE_ORACLE_INSTALL_BASE` or
/// `$HOME/.cache/blaze/oracle-out/ur/install/<install_base_key of UPSTREAM.json's current pin>`.
fn oracle_install_base(manifest: &std::path::Path) -> PathBuf {
    println!("cargo:rerun-if-env-changed=BLAZE_ORACLE_INSTALL_BASE");
    println!("cargo:rerun-if-env-changed=HOME");
    if let Some(p) = std::env::var_os("BLAZE_ORACLE_INSTALL_BASE") {
        return PathBuf::from(p);
    }
    let upstream = manifest.join("../../UPSTREAM.json");
    println!("cargo:rerun-if-changed={}", upstream.display());
    let text = std::fs::read_to_string(&upstream)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", upstream.display()));
    let value_after = |from: usize, key: &str| -> Option<String> {
        let i = text[from..].find(key)? + from + key.len();
        let rest = text[i..].trim_start().strip_prefix(':')?.trim_start();
        let rest = rest.strip_prefix('"')?;
        Some(rest[..rest.find('"')?].to_string())
    };
    let current = value_after(0, "\"current\"").expect("UPSTREAM.json has no \"current\"");
    let pin_key = format!("\"{current}\"");
    let mut search = 0;
    let pin_start = loop {
        let i = text[search..]
            .find(&pin_key)
            .map(|i| i + search)
            .unwrap_or_else(|| panic!("UPSTREAM.json has no pin for {current}"));
        let after = text[i + pin_key.len()..].trim_start();
        if after.starts_with(':') && after[1..].trim_start().starts_with('{') {
            break i;
        }
        search = i + pin_key.len();
    };
    let key = value_after(pin_start, "\"install_base_key\"")
        .unwrap_or_else(|| panic!("UPSTREAM.json pin {current} has no install_base_key"));
    let home = std::env::var_os("HOME").expect("HOME is not set; set BLAZE_ORACLE_INSTALL_BASE");
    PathBuf::from(home)
        .join(".cache/blaze/oracle-out/ur/install")
        .join(key)
}

/// Reads the stored (uncompressed) entries `names` of a zip file through its central directory.
fn read_stored_zip_entries(zip: &[u8], names: &[&str]) -> BTreeMap<String, Vec<u8>> {
    let u16_at = |i: usize| u16::from_le_bytes([zip[i], zip[i + 1]]) as usize;
    let u32_at =
        |i: usize| u32::from_le_bytes([zip[i], zip[i + 1], zip[i + 2], zip[i + 3]]) as usize;
    // End of central directory record (no zip64, no archive comment search beyond 64 KiB).
    let eocd = (0..zip.len().saturating_sub(21))
        .rev()
        .take(65536 + 22)
        .find(|&i| zip[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .expect("server jar: no end of central directory record");
    let entries = u16_at(eocd + 10);
    let mut p = u32_at(eocd + 16);
    let mut out = BTreeMap::new();
    for _ in 0..entries {
        assert_eq!(
            zip[p..p + 4],
            [0x50, 0x4b, 0x01, 0x02],
            "server jar: bad central header"
        );
        let method = u16_at(p + 10);
        let compressed = u32_at(p + 20);
        let size = u32_at(p + 24);
        let name_len = u16_at(p + 28);
        let extra_len = u16_at(p + 30);
        let comment_len = u16_at(p + 32);
        let local = u32_at(p + 42);
        let name = std::str::from_utf8(&zip[p + 46..p + 46 + name_len]).unwrap_or("");
        if names.contains(&name) {
            assert_eq!(
                method, 0,
                "server jar: {name} is compressed (only stored entries are read)"
            );
            assert_eq!(compressed, size);
            let data_start = local + 30 + u16_at(local + 26) + u16_at(local + 28);
            out.insert(
                name.to_string(),
                zip[data_start..data_start + size].to_vec(),
            );
        }
        p += 46 + name_len + extra_len + comment_len;
    }
    out
}

fn extract_jar_resources(manifest: &std::path::Path) {
    let install_base = oracle_install_base(manifest);
    let jar = install_base.join("A-server.jar");
    let bytes = std::fs::read(&jar).unwrap_or_else(|e| {
        panic!(
            "cannot read the oracle's server jar {}: {e}. Run the oracle once (scripts/oracle) \
             so that it extracts its install base, or set BLAZE_ORACLE_INSTALL_BASE.",
            jar.display()
        )
    });
    let entries = read_stored_zip_entries(&bytes, JAR_RESOURCES);
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let mut table = String::from(
        "/// The server jar resources Blaze's commands load (path in the jar, contents).\n\
         pub(crate) static JAR_RESOURCES: &[(&str, &[u8])] = &[\n",
    );
    for name in JAR_RESOURCES {
        let data = entries
            .get(*name)
            .unwrap_or_else(|| panic!("{} has no resource {name}", jar.display()));
        let path = out_dir.join("jar_resources").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, data).unwrap();
        writeln!(
            table,
            "    ({name:?}, include_bytes!(concat!(env!(\"OUT_DIR\"), \"/jar_resources/{name}\"))),"
        )
        .unwrap();
    }
    table.push_str("];\n");
    std::fs::write(out_dir.join("jar_resources.rs"), table).unwrap();
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    extract_jar_resources(&manifest);
}
