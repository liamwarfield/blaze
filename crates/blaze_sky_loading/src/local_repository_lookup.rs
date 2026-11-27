//! LOCAL_REPOSITORY_LOOKUP: which repository a directory belongs to.
//!
//! Java: com/google/devtools/build/lib/skyframe/{LocalRepositoryLookupFunction,
//! LocalRepositoryLookupValue}.java and packages/ErrorDeterminingRepositoryException.java. Since Bzlmod the function
//! always answers "the main repository".

use std::fmt;

use blaze_cmdline::RepositoryName;
use blaze_cmdline::repository_name::MAIN;
use blaze_skyframe::{ComputeError, Environment, FunctionHermeticity, Key, SkyFunctionName};
use blaze_vfs::{PathFragment, RootedPath};

use crate::sky_functions;

/// A value that represents a local repository lookup result.
// Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:33 LocalRepositoryLookupValue
#[derive(Clone, Debug)]
pub enum LocalRepositoryLookupValue {
    /// A successful lookup of the main repository.
    // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:105 LocalRepositoryLookupValue.MainRepositoryLookupValue
    MainRepository,
    /// A successful lookup of a local repository.
    // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:140 LocalRepositoryLookupValue.SuccessfulLocalRepositoryLookupValue
    Successful {
        repository_name: RepositoryName,
        path: PathFragment,
    },
    /// No repository was found, either local or the main repository.
    // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:189 LocalRepositoryLookupValue.NotFoundLocalRepositoryLookupValue
    NotFound,
}

impl LocalRepositoryLookupValue {
    // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:72 LocalRepositoryLookupValue.mainRepository
    pub fn main_repository() -> Self {
        LocalRepositoryLookupValue::MainRepository
    }

    // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:76 LocalRepositoryLookupValue.success
    pub fn success(repository_name: RepositoryName, path: PathFragment) -> Self {
        LocalRepositoryLookupValue::Successful {
            repository_name,
            path,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:81 LocalRepositoryLookupValue.notFound
    pub fn not_found() -> Self {
        LocalRepositoryLookupValue::NotFound
    }

    /// Returns `true` if the local repository lookup succeeded and `get_repository` will return a
    /// useful result.
    // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:89 LocalRepositoryLookupValue.exists
    pub fn exists(&self) -> bool {
        match self {
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:110 LocalRepositoryLookupValue.MainRepositoryLookupValue.exists
            LocalRepositoryLookupValue::MainRepository => true,
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:153 LocalRepositoryLookupValue.SuccessfulLocalRepositoryLookupValue.exists
            LocalRepositoryLookupValue::Successful { .. } => true,
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:193 LocalRepositoryLookupValue.NotFoundLocalRepositoryLookupValue.exists
            LocalRepositoryLookupValue::NotFound => false,
        }
    }

    /// Returns the name of the local repository contained in the directory which was looked up,
    /// the main repository if the directory is part of it, or panics if there was no repository
    /// found.
    // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:96 LocalRepositoryLookupValue.getRepository
    pub fn get_repository(&self) -> &RepositoryName {
        match self {
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:115 LocalRepositoryLookupValue.MainRepositoryLookupValue.getRepository
            LocalRepositoryLookupValue::MainRepository => &MAIN,
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:158 LocalRepositoryLookupValue.SuccessfulLocalRepositoryLookupValue.getRepository
            LocalRepositoryLookupValue::Successful {
                repository_name, ..
            } => repository_name,
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:198 LocalRepositoryLookupValue.NotFoundLocalRepositoryLookupValue.getRepository
            LocalRepositoryLookupValue::NotFound => {
                panic!("IllegalStateException: Repository was not found")
            }
        }
    }

    /// Returns the path to the local repository, or panics if there was no repository found.
    // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:102 LocalRepositoryLookupValue.getPath
    pub fn get_path(&self) -> PathFragment {
        match self {
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:120 LocalRepositoryLookupValue.MainRepositoryLookupValue.getPath
            LocalRepositoryLookupValue::MainRepository => PathFragment::empty_fragment(),
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:163 LocalRepositoryLookupValue.SuccessfulLocalRepositoryLookupValue.getPath
            LocalRepositoryLookupValue::Successful { path, .. } => path.clone(),
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:203 LocalRepositoryLookupValue.NotFoundLocalRepositoryLookupValue.getPath
            LocalRepositoryLookupValue::NotFound => {
                panic!("IllegalStateException: Repository was not found")
            }
        }
    }
}

impl fmt::Display for LocalRepositoryLookupValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:125 LocalRepositoryLookupValue.MainRepositoryLookupValue.toString
            LocalRepositoryLookupValue::MainRepository => f.write_str("MainRepositoryLookupValue"),
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:168 LocalRepositoryLookupValue.SuccessfulLocalRepositoryLookupValue.toString
            LocalRepositoryLookupValue::Successful {
                repository_name, ..
            } => write!(f, "SuccessfulLocalRepositoryLookupValue({repository_name})"),
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:208 LocalRepositoryLookupValue.NotFoundLocalRepositoryLookupValue.toString
            LocalRepositoryLookupValue::NotFound => {
                f.write_str("NotFoundLocalRepositoryLookupValue")
            }
        }
    }
}

/// Java: every main-repository value and every not-found value is equal; a successful lookup
/// compares the repository name only, not the path.
impl PartialEq for LocalRepositoryLookupValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:130 LocalRepositoryLookupValue.MainRepositoryLookupValue.equals
            (
                LocalRepositoryLookupValue::MainRepository,
                LocalRepositoryLookupValue::MainRepository,
            ) => true,
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:173 LocalRepositoryLookupValue.SuccessfulLocalRepositoryLookupValue.equals
            (
                LocalRepositoryLookupValue::Successful {
                    repository_name: a, ..
                },
                LocalRepositoryLookupValue::Successful {
                    repository_name: b, ..
                },
            ) => a == b,
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:213 LocalRepositoryLookupValue.NotFoundLocalRepositoryLookupValue.equals
            (LocalRepositoryLookupValue::NotFound, LocalRepositoryLookupValue::NotFound) => true,
            _ => false,
        }
    }
}

impl Eq for LocalRepositoryLookupValue {}

impl std::hash::Hash for LocalRepositoryLookupValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:136 LocalRepositoryLookupValue.MainRepositoryLookupValue.hashCode
            LocalRepositoryLookupValue::MainRepository => "MainRepositoryLookupValue".hash(state),
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:181 LocalRepositoryLookupValue.SuccessfulLocalRepositoryLookupValue.hashCode
            LocalRepositoryLookupValue::Successful {
                repository_name, ..
            } => repository_name.hash(state),
            // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:219 LocalRepositoryLookupValue.NotFoundLocalRepositoryLookupValue.hashCode
            LocalRepositoryLookupValue::NotFound => {
                "NotFoundLocalRepositoryLookupValue".hash(state)
            }
        }
    }
}

/// Key of LOCAL_REPOSITORY_LOOKUP: a directory. `Debug` prints `LOCAL_REPOSITORY_LOOKUP:<dir>`.
// Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:40 LocalRepositoryLookupValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct LocalRepositoryLookupKey(pub RootedPath);

impl LocalRepositoryLookupKey {
    // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:35 LocalRepositoryLookupValue.key
    pub fn key(directory: RootedPath) -> Self {
        LocalRepositoryLookupKey(directory)
    }
}

impl fmt::Debug for LocalRepositoryLookupKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", Self::FUNCTION, self.0)
    }
}

impl Key for LocalRepositoryLookupKey {
    type Value = LocalRepositoryLookupValue;
    const FUNCTION: SkyFunctionName = sky_functions::LOCAL_REPOSITORY_LOOKUP;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:138 SkyFunctions.LOCAL_REPOSITORY_LOOKUP (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java `equals` (see the `PartialEq` impl).
    fn values_equal(old: &LocalRepositoryLookupValue, new: &LocalRepositoryLookupValue) -> bool {
        old == new
    }
}

pub use crate::exceptions::ErrorDeterminingRepositoryException;

/// SkyFunction for [`LocalRepositoryLookupValue`]s.
// Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupFunction.java:28 LocalRepositoryLookupFunction.compute
pub fn local_repository_lookup_function(
    _sky_key: &LocalRepositoryLookupKey,
    _env: &mut Environment<'_>,
) -> Result<Option<LocalRepositoryLookupValue>, ComputeError> {
    // Implementation note: Although LocalRepositoryLookupValue.NOT_FOUND exists, it should never
    // be returned from this method.
    // TODO: #22208, #21515 - Figure out what to do here.
    Ok(Some(LocalRepositoryLookupValue::main_repository()))
}
