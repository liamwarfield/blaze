//! Port of `lib/actions/cache`: the action cache (`ActionCache`, `CompactPersistentActionCache`
//! and its on-disk format), plus the `lib/util` / `lib/vfs` pieces it is built on that are not
//! ported elsewhere yet (`PersistentMap`, `MapCodec`, `RecordOutputStream`, `VarInt`,
//! `DigestUtils.combineUnordered`).
//!
//! `VirtualActionInput` (also in Java's `actions/cache` package) is
//! [`crate::actions::virtual_action_input`].

pub mod action_cache;
pub mod compact_persistent_action_cache;
pub mod digest_utils;
pub mod map_codec;
pub mod metadata_digest_utils;
pub mod output_metadata_store;
pub mod persistent_map;
pub mod persistent_string_indexer;
pub mod postable_action_cache_stats;
pub mod record_output_stream;
pub mod var_int;

pub use action_cache::{
    ActionCache, ActionCacheEntry, ActionCacheEntryBuilder, ActionCacheStatistics, MissDetail,
    MissReason, OutputPermissions, SerializableTreeArtifactValue,
};
pub use compact_persistent_action_cache::CompactPersistentActionCache;
pub use metadata_digest_utils::MetadataDigestUtils;
pub use output_metadata_store::{IoOrInterruptedException, OutputMetadataStore};
pub use persistent_map::{PersistentMap, PersistentMapHooks, PersistentMapJournal};
pub use persistent_string_indexer::{PersistentStringIndexer, StringIndexer};
pub use postable_action_cache_stats::PostableActionCacheStats;
