//! LOCAL_REPOSITORY_LOOKUP: which repository a directory belongs to.
//!
//! Java: com/google/devtools/build/lib/skyframe/{LocalRepositoryLookupFunction,
//! LocalRepositoryLookupValue}.java and packages/ErrorDeterminingRepositoryException.java. Since Bzlmod the function
//! always answers "the main repository".

use std::fmt;

use blaze_cmdline::RepositoryName;
use blaze_cmdline::repository_name::MAIN;
use blaze_skyframe::{ComputeError, Environment, Key, SkyFunctionName};
use blaze_vfs::{PathFragment, RootedPath};

use crate::sky_functions;

/// A value that represents a local repository lookup result.
// Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:33 LocalRepositoryLookupValue
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
        !matches!(self, LocalRepositoryLookupValue::NotFound)
    }

    /// Returns the name of the local repository contained in the directory which was looked up,
    /// the main repository if the directory is part of it, or panics if there was no repository
    /// found.
    // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:96 LocalRepositoryLookupValue.getRepository
    pub fn get_repository(&self) -> &RepositoryName {
        match self {
            LocalRepositoryLookupValue::MainRepository => &*MAIN,
            LocalRepositoryLookupValue::Successful {
                repository_name, ..
            } => repository_name,
            LocalRepositoryLookupValue::NotFound => {
                panic!("IllegalStateException: Repository was not found")
            }
        }
    }

    /// Returns the path to the local repository, or panics if there was no repository found.
    // Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:102 LocalRepositoryLookupValue.getPath
    pub fn get_path(&self) -> PathFragment {
        match self {
            LocalRepositoryLookupValue::MainRepository => PathFragment::empty_fragment(),
            LocalRepositoryLookupValue::Successful { path, .. } => path.clone(),
            LocalRepositoryLookupValue::NotFound => {
                panic!("IllegalStateException: Repository was not found")
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
