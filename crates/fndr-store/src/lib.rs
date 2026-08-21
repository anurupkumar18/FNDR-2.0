//! SQLite schema and migrations, the single Lance writer, batched flush, compaction, rebuild, deletion-everywhere.
//!
//! Walking-skeleton state (T-109): a minimal records table plus FTS5 search,
//! enough to prove capture-to-retrieval end to end. The real schema v1 with
//! migrations is T-201; Lance and the flush writer are T-202.

mod skeleton;

pub use skeleton::{SearchHit, SkeletonStore, StoreError};
