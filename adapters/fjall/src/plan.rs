//! Pure, IO-free append planner — the single source of truth for fjall's
//! write path.
//!
//! Both [`RawEventStore::append`](crate::store) and the
//! [`AtomicAppend`](mnesis_store::import::AtomicAppend) impl reduce to the same
//! per-stream work: validate that a run's versions are strictly sequential from
//! the stream's current version, then encode each event's primary key, `$all`
//! key, and 16-byte-aligned wire frame while assigning a running
//! [`GlobalSeq`](crate::GlobalSeq). None of that touches fjall — it is a pure
//! function of `(current_version, current_global, id, envelopes)`, so it lives
//! here, unit-tested with no database, exactly as `mnesis-postgres` factors its
//! `narrow_inserts` out of `append`. The append *contract* (optimistic
//! concurrency + strict-sequential versions) is validated once in the kernel
//! [`validate_append_versions`](mnesis_store::store::validate_append_versions)
//! before this core runs; `plan_run` re-derives each version from a running
//! counter for the key codec rather than trusting the envelope's own field.
//!
//! The two public methods differ only in their *error domain* and in the
//! single-stream-vs-cross-run validation that wraps this core: `append` maps a
//! [`PlanError`] into [`AppendError`](mnesis_store::error::AppendError); the
//! atomic path owns its own cross-run head/projected-head/non-injective-route
//! check (index-based conflicts) and then calls [`plan_run`] purely to stage.

use bytes::Bytes;
use mnesis::ErrorId;
use mnesis_store::PendingBatch;
use mnesis_store::StreamKey;
use mnesis_store::wire;

use crate::error::reason_label;
use crate::wire_key::{encode_event_key, encode_global_key};

/// A validated, encoded event row ready to `tx.insert` into the `events` and
/// `events_global` partitions.
///
/// Holding a `StagedRow` is proof the event passed the strict-sequential check
/// and encoded cleanly — it owns its key bytes and frame and borrows nothing,
/// so the IO shell that consumes it is a mechanical insert loop.
#[derive(Debug)]
pub struct StagedRow {
    /// `events` partition key: `[u16 BE id_len][id_bytes][u64 BE version]`.
    pub event_key: Vec<u8>,
    /// `events_global` partition key:
    /// `[u64 BE global_seq][u16 BE id_len][id_bytes][u64 BE version]`.
    pub global_key: Vec<u8>,
    /// The 16-byte-aligned V2 wire frame (the value written to both partitions).
    pub frame: Bytes,
}

/// The result of planning one stream's append run.
#[derive(Debug)]
pub struct PlannedRun {
    /// The staged rows in version order — non-empty, because
    /// [`PendingBatch`] is.
    pub rows: Vec<StagedRow>,
    /// The stream's new version counter.
    pub new_version: u64,
    /// The position stamped on the run's LAST event — what
    /// [`RawEventStore::append`](mnesis_store::RawEventStore::append) returns
    /// (#330). Always `>= current_global + 1`.
    pub ending_global: u64,
}

/// Neutral planner failure — each caller maps it into its own error domain
/// (rule 3: one variant = one failure domain; overflow is never a conflict).
#[derive(Debug)]
pub enum PlanError {
    /// The stream version sequence would advance past `u64::MAX`.
    VersionOverflow,
    /// The store-global sequence would advance past `u64::MAX`.
    GlobalSeqOverflow,
    /// An event failed to encode (over-long id, or a wire-frame build failure).
    InvalidInput { version: u64, reason: ErrorId<128> },
}

/// Plan one stream's append run: validate strict-sequential versions from
/// `current_version`, then encode + stage each event assigning a running
/// `GlobalSeq` from `current_global`. Pure — no fjall, no `tx`.
///
/// `current_version` is the stream's current max (0 = fresh stream); the run's
/// first event must be version `current_version + 1`. `current_global` is the
/// store-wide counter; the first staged event is stamped `current_global + 1`.
pub fn plan_run(
    current_version: u64,
    current_global: u64,
    id: &StreamKey,
    envelopes: PendingBatch<'_>,
) -> Result<PlannedRun, PlanError> {
    let id_bytes = id.as_ref();
    let mut version = current_version;
    let mut global_seq = current_global;
    let mut rows = Vec::with_capacity(envelopes.len().get());

    for env in envelopes {
        // `version` is the validated, strictly-sequential successor of
        // `current_version`. The kernel `validate_append_versions` (single-stream
        // path) or `validate_atomic_writes` (atomic path) already proved the
        // batch is `current_version+1, +2, …`, so we re-derive it here from the
        // running counter rather than trusting the envelope's own version field
        // — a stray envelope version can't reach the key codec. `checked_add`
        // keeps the arithmetic overflow-safe (rule 2); it is unreachable once the
        // kernel validator has passed.
        version = version.checked_add(1).ok_or(PlanError::VersionOverflow)?;
        global_seq = global_seq
            .checked_add(1)
            .ok_or(PlanError::GlobalSeqOverflow)?;

        let event_key =
            encode_event_key(id_bytes, version).map_err(|e| PlanError::InvalidInput {
                version,
                reason: reason_label(&e),
            })?;
        let frame = wire::encode_frame(
            env.schema_version_value(),
            &env.event_type_value(),
            &env.payload_value(),
            env.metadata_value().as_ref(),
        )
        .map_err(|e| PlanError::InvalidInput {
            version,
            reason: reason_label(&e),
        })?;
        // Defensively-unreachable arm: the id already passed the same u16
        // length gate in `encode_event_key` above — but it stays typed (rule 3),
        // never an unwrap.
        let global_key = encode_global_key(global_seq, id_bytes, version).map_err(|e| {
            PlanError::InvalidInput {
                version,
                reason: reason_label(&e),
            }
        })?;

        rows.push(StagedRow {
            event_key,
            global_key,
            frame: frame.value,
        });
    }

    // After the loop `version == current_version + envelopes.len()`. The batch is
    // non-empty, so the loop ran at least once and both counters advanced.
    let new_version = version;
    Ok(PlannedRun {
        rows,
        new_version,
        ending_global: global_seq,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test code")]
#[allow(clippy::panic, reason = "test code")]
mod tests {
    use super::*;
    use mnesis::Version;
    use mnesis_store::envelope::pending_envelope;

    fn sk() -> StreamKey {
        StreamKey::from_slice(b"s")
    }

    /// A minimal valid envelope at `version` (>= 1).
    fn env(version: u64) -> mnesis_store::PendingEnvelope {
        pending_envelope(Version::new(version).unwrap())
            .event_type("E")
            .payload(b"p".as_slice())
            .build()
            .unwrap()
    }

    fn batch(evs: &[mnesis_store::PendingEnvelope]) -> mnesis_store::PendingBatch<'_> {
        mnesis_store::PendingBatch::new(evs).expect("test runs are non-empty")
    }

    // 1. Sequence/protocol — happy paths ------------------------------------

    #[test]
    fn fresh_stream_three_events_stamps_versions_and_global() {
        let evs = [env(1), env(2), env(3)];
        let p = plan_run(0, 0, &sk(), batch(&evs)).unwrap();
        assert_eq!(p.rows.len(), 3);
        assert_eq!(p.new_version, 3);
        assert_eq!(p.ending_global, 3);
    }

    #[test]
    fn existing_stream_continues_version_and_global() {
        // current_version = 5, current_global = 40, run [6, 7]
        let evs = [env(6), env(7)];
        let p = plan_run(5, 40, &sk(), batch(&evs)).unwrap();
        assert_eq!(p.rows.len(), 2);
        assert_eq!(p.new_version, 7);
        assert_eq!(p.ending_global, 42);
    }

    // REMOVED `empty_batch_stages_nothing_and_leaves_counters` (#330): `plan_run`
    // takes a non-empty `PendingBatch`, so an empty run is unrepresentable.

    // 2. Defensive boundary — overflow paths. Sequence conflicts now live in the
    //    kernel `validate_append_versions` (see `crates/store/src/store.rs`); the
    //    `store-testing` harness still drives the full contract through `append`.

    #[test]
    fn version_overflow_at_ceiling_is_version_overflow_not_conflict() {
        // current_version = u64::MAX → the first successor overflows. This must
        // be VersionOverflow, never Conflict (rule 3: overflow is not a retry-
        // eligible conflict). `plan_run` still re-derives the version via
        // `checked_add(1)` for the key codec, so it surfaces here even though the
        // kernel validated the sequence.
        let evs = [env(1)];
        assert!(matches!(
            plan_run(u64::MAX, 0, &sk(), batch(&evs)).unwrap_err(),
            PlanError::VersionOverflow
        ));
    }

    #[test]
    fn global_seq_overflow_at_ceiling_is_global_seq_overflow() {
        // current_global = u64::MAX → stamping the first event overflows.
        let evs = [env(1)];
        assert!(matches!(
            plan_run(0, u64::MAX, &sk(), batch(&evs)).unwrap_err(),
            PlanError::GlobalSeqOverflow
        ));
    }

    // 3. Staged bytes are the real encoders --------------------------------

    #[test]
    fn staged_keys_match_the_wire_key_codecs() {
        let evs = [env(1)];
        let p = plan_run(0, 0, &sk(), batch(&evs)).unwrap();
        // event_key = [u16 id_len][id][u64 version];
        // global_key = [gseq][u16 id_len][id][ver].
        assert_eq!(p.rows[0].event_key, encode_event_key(b"s", 1).unwrap());
        assert_eq!(p.rows[0].global_key, encode_global_key(1, b"s", 1).unwrap());
        assert!(!p.rows[0].frame.is_empty());
    }
}
