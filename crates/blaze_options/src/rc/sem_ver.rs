//! Port of `src/main/cpp/sem_ver.cc`.

use std::cmp::Ordering;
use std::sync::LazyLock;

use regex::Regex;

// Java: src/main/cpp/sem_ver.cc:28 kSemverRe
static SEMVER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$").unwrap()
});

// Java: src/main/cpp/sem_ver.h:27 SemVer
/// A semantic version (semver.org).
#[derive(Clone, Debug)]
pub struct SemVer {
    major: i32,
    minor: i32,
    patch: i32,
    prerelease: String,
    buildmetadata: String,
}

impl SemVer {
    // Java: src/main/cpp/sem_ver.cc:32 SemVer::Parse
    pub fn parse(v: &str) -> Option<SemVer> {
        let m = SEMVER_RE.captures(v)?;
        // RE2::FullMatch into an int fails on overflow.
        Some(SemVer {
            major: m[1].parse().ok()?,
            minor: m[2].parse().ok()?,
            patch: m[3].parse().ok()?,
            prerelease: m.get(4).map(|x| x.as_str().to_string()).unwrap_or_default(),
            buildmetadata: m.get(5).map(|x| x.as_str().to_string()).unwrap_or_default(),
        })
    }

    // Java: src/main/cpp/sem_ver.cc:46 SemVer::NextMajorVersion
    pub fn next_major_version(&self) -> SemVer {
        SemVer {
            major: self.major + 1,
            minor: 0,
            patch: 0,
            prerelease: String::new(),
            buildmetadata: String::new(),
        }
    }

    // Java: src/main/cpp/sem_ver.cc:50 SemVer::NextMinorVersion
    pub fn next_minor_version(&self) -> SemVer {
        SemVer {
            major: self.major,
            minor: self.minor + 1,
            patch: 0,
            prerelease: String::new(),
            buildmetadata: String::new(),
        }
    }

    pub fn build_metadata(&self) -> &str {
        &self.buildmetadata
    }

    // Java: src/main/cpp/sem_ver.cc:54 SemVer::Compare
    fn compare(&self, other: &SemVer) -> i32 {
        let major_diff = self.major - other.major;
        if major_diff != 0 {
            return major_diff;
        }
        let minor_diff = self.minor - other.minor;
        if minor_diff != 0 {
            return minor_diff;
        }
        let patch_diff = self.patch - other.patch;
        if patch_diff != 0 {
            return patch_diff;
        }
        Self::compare_prerelease(&self.prerelease, &other.prerelease)
    }

    // Java: src/main/cpp/sem_ver.cc:72 SemVer::ComparePrerelease
    fn compare_prerelease(x: &str, y: &str) -> i32 {
        if x == y {
            return 0;
        }
        if x.is_empty() {
            return 1;
        }
        if y.is_empty() {
            return -1;
        }
        let xv: Vec<&str> = x.split('.').collect();
        let yv: Vec<&str> = y.split('.').collect();
        let mut i = 0;
        while i != xv.len() && i != yv.len() {
            let part_x = xv[i];
            let part_y = yv[i];
            i += 1;
            if part_x != part_y {
                let ix = part_x.parse::<i32>().ok();
                let iy = part_y.parse::<i32>().ok();
                if ix.is_some() != iy.is_some() {
                    return if ix.is_some() { -1 } else { 1 };
                }
                if let (Some(ix), Some(iy)) = (ix, iy) {
                    return if ix < iy { -1 } else { 1 };
                }
                return if part_x < part_y { -1 } else { 1 };
            }
        }
        if i == xv.len() { -1 } else { 1 }
    }
}

impl PartialEq for SemVer {
    fn eq(&self, other: &Self) -> bool {
        self.compare(other) == 0
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.compare(other).cmp(&0))
    }
}
