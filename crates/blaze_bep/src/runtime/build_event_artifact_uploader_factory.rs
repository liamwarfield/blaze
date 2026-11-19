//! Port of `BuildEventArtifactUploaderFactory` and `BuildEventArtifactUploaderFactoryMap`.

use std::collections::BTreeMap;
use std::sync::Arc;

use indexmap::IndexMap;

use crate::buildeventstream::artifact_uploader::{
    BuildEventArtifactUploader, LocalFilesArtifactUploader,
};

/// A factory for [`BuildEventArtifactUploader`].
///
/// PORT: Java's `create` takes the `CommandEnvironment`; the factories of Blaze hold what they
/// need from it, so `create` takes no argument. `InvalidPackagePathSymlinkException` is not
/// thrown by any factory Blaze has.
// Java: com/google/devtools/build/lib/runtime/BuildEventArtifactUploaderFactory.java:21 BuildEventArtifactUploaderFactory
pub trait BuildEventArtifactUploaderFactory: Send + Sync {
    /// Returns a new instance of a [`BuildEventArtifactUploader`]. The caller is responsible for
    /// calling [`BuildEventArtifactUploader::release`] on the returned instance.
    // Java: com/google/devtools/build/lib/runtime/BuildEventArtifactUploaderFactory.java:30 BuildEventArtifactUploaderFactory.create
    fn create(&self) -> Arc<dyn BuildEventArtifactUploader>;

    /// If the factory reuses a `BuildEventArtifactUploader` across commands, tear down that
    /// uploader now to prepare for *blaze* shutdown.
    // Java: com/google/devtools/build/lib/runtime/BuildEventArtifactUploaderFactory.java:37 BuildEventArtifactUploaderFactory.shutdown
    fn shutdown(&self) {}
}

/// `BuildEventArtifactUploaderFactory.LOCAL_FILES_UPLOADER_FACTORY`.
// Java: com/google/devtools/build/lib/runtime/BuildEventArtifactUploaderFactory.java:23 BuildEventArtifactUploaderFactory.LOCAL_FILES_UPLOADER_FACTORY
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalFilesUploaderFactory;

impl BuildEventArtifactUploaderFactory for LocalFilesUploaderFactory {
    fn create(&self) -> Arc<dyn BuildEventArtifactUploader> {
        Arc::new(LocalFilesArtifactUploader)
    }
}

/// Selects between multiple available upload strategies.
// Java: com/google/devtools/build/lib/runtime/BuildEventArtifactUploaderFactoryMap.java:24 BuildEventArtifactUploaderFactoryMap
#[derive(Clone, Default)]
pub struct BuildEventArtifactUploaderFactoryMap {
    /// Sorted by name (the builder's `TreeMap`).
    uploaders: IndexMap<String, Arc<dyn BuildEventArtifactUploaderFactory>>,
}

impl BuildEventArtifactUploaderFactoryMap {
    // Java: com/google/devtools/build/lib/runtime/BuildEventArtifactUploaderFactoryMap.java:33 BuildEventArtifactUploaderFactoryMap.select
    pub fn select(&self, name: Option<&str>) -> Arc<dyn BuildEventArtifactUploaderFactory> {
        if name.is_none() && !self.uploaders.is_empty() {
            // TODO(b/110235226): We currently choose the strategy with alphabetically first
            // strategy, which happens to be backwards-compatible; we need to set
            // experimental_build_event_upload_strategy to appropriate default values instead,
            // and then make it an error to pass null.
            return self.uploaders.values().next().expect("not empty").clone();
        }
        name.and_then(|name| self.uploaders.get(name))
            .cloned()
            .unwrap_or_else(|| Arc::new(LocalFilesUploaderFactory))
    }
}

/// Builder class for [`BuildEventArtifactUploaderFactoryMap`].
// Java: com/google/devtools/build/lib/runtime/BuildEventArtifactUploaderFactoryMap.java:46 BuildEventArtifactUploaderFactoryMap.Builder
#[derive(Default)]
pub struct BuildEventArtifactUploaderFactoryMapBuilder {
    uploaders: BTreeMap<String, Arc<dyn BuildEventArtifactUploaderFactory>>,
}

impl BuildEventArtifactUploaderFactoryMapBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventArtifactUploaderFactoryMap.java:52 Builder.add
    pub fn add(mut self, name: &str, uploader: Arc<dyn BuildEventArtifactUploaderFactory>) -> Self {
        self.uploaders.insert(name.to_string(), uploader);
        self
    }

    // Java: com/google/devtools/build/lib/runtime/BuildEventArtifactUploaderFactoryMap.java:57 Builder.build
    pub fn build(self) -> BuildEventArtifactUploaderFactoryMap {
        BuildEventArtifactUploaderFactoryMap {
            uploaders: self.uploaders.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buildeventstream::artifact_uploader::PathConverterFuture;

    struct Named(&'static str);

    struct NamedUploader(&'static str);

    impl BuildEventArtifactUploader for NamedUploader {
        fn upload(
            &self,
            _files: std::collections::HashMap<blaze_vfs::Path, crate::buildeventstream::LocalFile>,
        ) -> PathConverterFuture {
            PathConverterFuture::immediate_cancelled()
        }
        fn may_be_slow(&self) -> bool {
            self.0 != "local"
        }
    }

    impl BuildEventArtifactUploaderFactory for Named {
        fn create(&self) -> Arc<dyn BuildEventArtifactUploader> {
            Arc::new(NamedUploader(self.0))
        }
    }

    #[test]
    fn select_null_is_alphabetically_first_and_unknown_is_local() {
        let map = BuildEventArtifactUploaderFactoryMapBuilder::new()
            .add("zeta", Arc::new(Named("zeta")))
            .add("remote", Arc::new(Named("remote")))
            .build();
        assert!(map.select(None).create().may_be_slow());
        assert!(map.select(Some("remote")).create().may_be_slow());
        assert!(!map.select(Some("local")).create().may_be_slow());
        let empty = BuildEventArtifactUploaderFactoryMapBuilder::new().build();
        assert!(!empty.select(None).create().may_be_slow());
    }
}
