//! Compaction and version-prune scheduler (T-204). LanceDB's MVCC commit
//! model means every batched flush (`LanceWriter::flush_once`) both adds a
//! new, small fragment and bumps a new dataset version; left unmanaged under
//! FNDR's write pattern (many small batches every 30 to 60 seconds, ADR-002)
//! both grow without bound.
//!
//! The T-208 spike (`docs/spikes/T-208-lance-findings.md`) measured the
//! fix, and the binding correction is repeated in
//! `.claude/skills/fndr-v2-engineering/references/lessons.md` (2026-08-21,
//! "Lance default prune reclaims nothing for our write pattern") because it
//! would otherwise cost a shipped-but-useless scheduler: `OptimizeAction`'s
//! default action (`All`, which `optimize()`'s own docs suggest reaching
//! for) compacts fragments correctly but pairs it with a 7-day prune window,
//! and every FNDR version is younger than that, so disk never comes back --
//! the spike measured it *rising* (compaction keeps the superseded
//! fragments' versions alive while writing the merged one). Only an
//! explicit prune (`older_than = 0`, `delete_unverified = true`) reclaimed
//! the disk (331 MB in 57 ms in the spike). `delete_unverified` is
//! documented upstream as safe only single-process; FNDR's app-wide
//! instance lock over the data directory (ADR-002 action item 4) is exactly
//! that guarantee, and `LanceWriter` is already the only writer for this
//! table (ADR-002), so pruning through it is coordination, not a second
//! writer.

use lancedb::table::optimize::Duration as LanceDuration;
use lancedb::table::{CompactionOptions, OptimizeAction, Table};

use crate::{FlushError, LanceWriter};

/// Maintenance cadence bounds: the contract for whatever scheduler drives
/// `LanceWriter::compact_and_prune`, mirroring the `FLUSH_INTERVAL_SECS_*`
/// convention in `lance_writer.rs`. Compaction and prune are not free (T-208
/// spike: roughly 511 ms compact plus 57 ms explicit prune at 100k rows), so
/// maintenance is expected to run far less often than the 30-60 second flush
/// cadence -- doing it on every flush would waste that cost on tables with
/// nothing new to compact.
pub const MAINTENANCE_INTERVAL_SECS_MIN: u64 = 15 * 60;
pub const MAINTENANCE_INTERVAL_SECS_MAX: u64 = 60 * 60;

/// What one `compact_and_prune` call did, aggregated from Lance's own
/// compaction and prune metrics. `bytes_removed` is Lance's self-reported
/// figure, kept here for observability; it is deliberately not what proves
/// disk actually returned -- per the lessons.md entry this module exists to
/// satisfy, that proof comes from measuring the filesystem independently
/// (`crates/fndr-store/tests/compaction.rs`), not from trusting an API's own
/// number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionReport {
    pub fragments_removed: usize,
    pub fragments_added: usize,
    pub versions_removed: u64,
    pub bytes_removed: u64,
}

/// Never a silent no-op (invariant: no silent degradation). A table that has
/// never been flushed is a distinct, observable outcome from one maintenance
/// actually ran against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionOutcome {
    /// Nothing has ever been flushed to this table, so there is nothing to
    /// compact or prune. Not an error: a fresh vault has no derived index
    /// yet (T-208 finding 4).
    NoTable,
    Ran(CompactionReport),
}

impl LanceWriter {
    /// Compact small fragments, then explicitly prune old versions, on the
    /// table `table_name` (an `EmbeddingSpec::lance_table`, matching the
    /// name `flush_once` writes to). Order matters: compacting first merges
    /// the small per-flush fragments into one, and pruning immediately after
    /// reclaims the space the now-superseded fragments and the versions that
    /// referenced them held. Pruning first would remove version history
    /// compaction still needs to read from.
    ///
    /// A missing table is `Ok(CompactionOutcome::NoTable)`, not an error --
    /// mirroring `delete_records`'s treatment of the same case. Any other
    /// Lance failure is returned, never swallowed: a caller that skips a
    /// failed maintenance cycle must decide that on a typed error, not on an
    /// inferred absence of effect.
    pub async fn compact_and_prune(
        &self,
        table_name: &str,
    ) -> Result<CompactionOutcome, FlushError> {
        let db = lancedb::connect(&self.uri).execute().await?;
        let table = match db.open_table(table_name).execute().await {
            Ok(table) => table,
            Err(lancedb::Error::TableNotFound { .. }) => return Ok(CompactionOutcome::NoTable),
            Err(error) => return Err(error.into()),
        };

        let compaction = table
            .optimize(OptimizeAction::Compact {
                options: CompactionOptions::default(),
                remap_options: None,
            })
            .await?
            .compaction
            .unwrap_or_default();

        // The binding correction from the T-208 spike: Lance's default
        // prune window is 7 days and refuses unverified files newer than
        // that, so it is a no-op under our cadence. Ask explicitly instead.
        let prune = table
            .optimize(OptimizeAction::Prune {
                older_than: Some(LanceDuration::zero()),
                delete_unverified: Some(true),
                error_if_tagged_old_versions: None,
            })
            .await?
            .prune
            .unwrap_or_default();

        Ok(CompactionOutcome::Ran(CompactionReport {
            fragments_removed: compaction.fragments_removed,
            fragments_added: compaction.fragments_added,
            versions_removed: prune.old_versions,
            bytes_removed: prune.bytes_removed,
        }))
    }
}
