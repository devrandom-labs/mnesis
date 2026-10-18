//! The default CBOR backup box — the bytes↔sections travel codec.
//!
//! Connects raw export to [`EventImporter`](crate::import::EventImporter).
//! Format 2 is a CBOR sequence with a header, section headings, event blocks
//! and a required completion record. Normal [`decode_chunk`] rejects incomplete
//! artifacts; [`salvage_chunk`] makes partial/legacy recovery explicit.
//! See `docs/backup-format.md` for the wire format and migration procedure.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use core::convert::Infallible;

use bytes::Bytes;
use futures::Stream;
use futures::StreamExt as _;
use minicbor::Decoder;
use minicbor::Encoder;
use minicbor::data::Type;
use thiserror::Error;

use crate::envelope::PersistedEnvelope;
use crate::import::{ImportBlock, StreamSection};
use crate::value::SchemaVersion;
use mnesis::Version;

const MAGIC: &[u8] = b"nxch";
const FORMAT_VERSION: u32 = 2;
const COMPLETION_TAG: u64 = 60000;

/// A box-layer decode failure.
///
/// Not [`ImportError`](crate::import::ImportError) — the box has no store error
/// or id type. Decode-only: write-path failures are [`WriteError`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ChunkError {
    /// Framing violates a structural rule. Parser failures retain their
    /// original source in [`Self::Decode`].
    #[error("malformed chunk: {0}")]
    Malformed(&'static str),
    /// CBOR parsing failed, retaining the parser's original error.
    #[error("{context}: {source}")]
    Decode {
        context: &'static str,
        #[source]
        source: minicbor::decode::Error,
    },
    /// A block's fields cannot form a valid persisted envelope.
    #[error("invalid backup event: {0}")]
    Event(#[from] BackupEventError),
    /// The backup has no valid completion evidence.
    #[error(transparent)]
    Completion(#[from] CompletionError),
    /// A complete restore cannot accept a corrupt block.
    #[error("backup contains a corrupt event block")]
    CorruptBlock,
}

/// Invalid event data in a CRC-valid backup block.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BackupEventError {
    /// Event version zero is invalid.
    #[error("event version must be nonzero")]
    VersionZero,
    /// A field violates the envelope value contract.
    #[error(transparent)]
    Value(#[from] crate::value::ValueError),
    /// Combined event bytes exceed representable offsets.
    #[error("event byte length overflow")]
    LengthOverflow,
    /// An offset cannot be represented.
    #[error("event offset cannot be represented: {0}")]
    Offset(#[from] core::num::TryFromIntError),
    /// The aligned wire frame cannot represent the event.
    #[error(transparent)]
    Wire(#[from] crate::wire::WireError),
    /// Envelope construction rejected the event.
    #[error(transparent)]
    Envelope(#[from] crate::envelope::EnvelopeError),
}

/// Why a backup cannot be treated as complete.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CompletionError {
    /// Format 1 carries no completion evidence.
    #[error("legacy format 1 requires explicit salvage")]
    Legacy,
    /// The format-2 completion record is absent or truncated.
    #[error("backup completion record is missing or truncated")]
    Missing,
    /// Completion CBOR is structurally invalid.
    #[error("malformed completion: {0}")]
    Decode(#[source] minicbor::decode::Error),
    /// The completion tag or array shape is invalid.
    #[error("invalid completion record shape")]
    Shape,
    /// Declared counts differ from parsed counts.
    #[error(
        "completion counts differ: sections {declared_sections}/{sections}, events {declared_events}/{events}"
    )]
    Counts {
        declared_sections: u64,
        sections: u64,
        declared_events: u64,
        events: u64,
    },
    /// Declared prefix length differs from its actual length.
    #[error("completion prefix length differs: {declared}/{actual}")]
    Length { declared: u64, actual: u64 },
    /// The prefix was altered or damaged.
    #[error("completion prefix checksum differs")]
    Checksum,
    /// Bytes follow the completion record.
    #[error("bytes follow the completion record")]
    TrailingBytes,
    /// A count or position cannot be represented.
    #[error("backup completion count cannot be represented: {0}")]
    CountConversion(#[source] core::num::TryFromIntError),
    /// Count accumulation overflowed.
    #[error("backup completion count overflow")]
    CountOverflow,
}

/// Explicit partial-recovery result. Even legacy artifacts keep their provenance.
/// Inspect [`Self::completion`] before choosing to recover its sections.
#[derive(Debug)]
pub struct SalvagedChunk {
    header: ChunkHeader,
    sections: Vec<StreamSection>,
    completion: Result<(), CompletionError>,
}

impl SalvagedChunk {
    /// Producer and original format version.
    pub const fn header(&self) -> &ChunkHeader {
        &self.header
    }
    /// Whether completion framing, counts, length and prefix CRC all verified.
    /// This does not imply that individual blocks are free of corruption.
    ///
    /// # Errors
    /// Returns the retained reason that completion could not be established.
    pub const fn completion(&self) -> Result<(), &CompletionError> {
        self.completion.as_ref().copied()
    }
    /// Inspect recovered sections, including any corrupt blocks.
    pub fn sections(&self) -> &[StreamSection] {
        &self.sections
    }
    /// Explicitly accept recovery content that may be incomplete or corrupt.
    pub fn into_sections_for_partial_recovery(self) -> Vec<StreamSection> {
        self.sections
    }
}

/// A write-path failure, generic over the sink's write error `E` (`W::Error`).
///
/// Keeps sink failures, scratch serialization failures and writer state errors
/// distinct. A generic sink can fail partway through any item.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WriteError<E> {
    /// minicbor serialization or the sink failed while writing a chunk item.
    #[error("chunk write failed: {0}")]
    Encode(#[from] minicbor::encode::Error<ChunkSinkError<E>>),
    /// Scratch serialization failed before writing a block.
    #[error("block serialization failed: {0}")]
    Scratch(#[source] minicbor::encode::Error<Infallible>),
    /// A previous write, source failure or canceled drain left an incomplete chunk.
    #[error("chunk writer is poisoned")]
    Poisoned,
    /// A section or event count cannot be represented by the format.
    #[error("chunk count overflow")]
    CountOverflow,
}

/// A `SectionWriter::try_extend` failure: the two domains it spans, kept
/// distinct (CLAUDE rule 3) — a read failure is never reported as a write
/// failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SectionError<E, R> {
    /// Writing a block into the chunk failed.
    #[error(transparent)]
    Write(#[from] WriteError<E>),
    /// The source event stream yielded an error.
    #[error("event stream read failed: {0}")]
    Read(#[source] R),
}

/// Failure while forwarding bytes to the backup sink.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ChunkSinkError<E> {
    /// The underlying sink failed; its original error is retained.
    #[error("backup sink failed: {0}")]
    Sink(#[source] E),
    /// A write length cannot be represented by the format.
    #[error("backup write length cannot be represented: {0}")]
    LengthConversion(#[source] core::num::TryFromIntError),
    /// The byte length cannot be represented by the format.
    #[error("backup byte length overflow")]
    LengthOverflow,
}

#[derive(Debug)]
struct TrackedSink<W> {
    inner: W,
    bytes: u64,
    crc: u32,
}

impl<W: minicbor::encode::Write> minicbor::encode::Write for TrackedSink<W> {
    type Error = ChunkSinkError<W::Error>;

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        let len = u64::try_from(bytes.len()).map_err(ChunkSinkError::LengthConversion)?;
        let next = self
            .bytes
            .checked_add(len)
            .ok_or(ChunkSinkError::LengthOverflow)?;
        self.inner.write_all(bytes).map_err(ChunkSinkError::Sink)?;
        self.bytes = next;
        self.crc = crc32c::crc32c_append(self.crc, bytes);
        Ok(())
    }
}

#[derive(Debug)]
enum WriterState {
    Ready,
    Poisoned,
}

/// The decoded chunk header — what [`decode_header`] returns.
#[derive(Debug, Clone)]
pub struct ChunkHeader {
    /// The chunk format version (`2` for newly written chunks).
    pub format_version: u32,
    /// The chunk-level producer/device id, if the encoder recorded one.
    pub origin: Option<Bytes>,
}

/// Wire shape of the header map `{0: magic, 1: format_version, 2: origin?}`.
#[derive(minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
struct HeaderRepr<'a> {
    #[n(0)]
    #[cbor(with = "minicbor::bytes")]
    magic: &'a [u8],
    #[n(1)]
    format_version: u32,
    #[n(2)]
    #[cbor(with = "minicbor::bytes")]
    origin: Option<&'a [u8]>,
}

// Do not let derived decoding silently accept duplicate fields or indefinite
// maps. Work and storage follow bytes actually parsed, never declared lengths.
fn decode_map<'a, T: minicbor::Decode<'a, ()>>(
    d: &mut Decoder<'a>,
) -> Result<T, minicbor::decode::Error> {
    let mut probe = d.clone();
    let length = probe
        .map()?
        .ok_or_else(|| minicbor::decode::Error::message("indefinite backup map"))?;
    let mut keys = BTreeSet::new();
    for _ in 0..length {
        let key = probe.u32()?;
        if !keys.insert(key) {
            return Err(minicbor::decode::Error::message("duplicate backup map key"));
        }
        probe.skip()?;
    }
    d.decode()
}

fn validate_header(magic: &[u8], format_version: u32) -> Result<(), ChunkError> {
    if magic != MAGIC {
        return Err(ChunkError::Malformed("bad magic"));
    }
    if !matches!(format_version, 1 | FORMAT_VERSION) {
        return Err(ChunkError::Malformed("unknown format version"));
    }
    Ok(())
}

/// Decode just the header — a cheap peek that validates magic + format version
/// and returns the chunk-level origin without parsing the body.
///
/// # Errors
/// Returns [`ChunkError::Malformed`] on bad magic or an unknown version,
/// and [`ChunkError::Decode`] on unreadable header bytes. Accepting a header
/// (format 1 or 2) does not establish backup completeness.
pub fn decode_header(bytes: &[u8]) -> Result<ChunkHeader, ChunkError> {
    let repr: HeaderRepr =
        decode_map(&mut Decoder::new(bytes)).map_err(|source| ChunkError::Decode {
            context: "unreadable header",
            source,
        })?;
    validate_header(repr.magic, repr.format_version)?;
    Ok(ChunkHeader {
        format_version: repr.format_version,
        origin: repr.origin.map(Bytes::copy_from_slice),
    })
}

/// A streaming format-2 backup writer.
///
/// Only [`Self::finish`] writes completion evidence. Source errors, sink errors
/// and cancellation of a drain poison the writer so a partial export cannot
/// subsequently be completed. Completion does not flush or sync the sink;
/// callers must apply their storage durability policy.
#[derive(Debug)]
pub struct ChunkWriter<W> {
    enc: Encoder<TrackedSink<W>>,
    state: WriterState,
    sections: u64,
    events: u64,
}

impl<W: minicbor::encode::Write> ChunkWriter<W> {
    /// Emit the header with an optional producer/device id.
    ///
    /// # Errors
    /// Returns [`WriteError`] if the header cannot be written.
    pub fn new(sink: W, origin: Option<&[u8]>) -> Result<Self, WriteError<W::Error>> {
        let mut enc = Encoder::new(TrackedSink {
            inner: sink,
            bytes: 0,
            crc: 0,
        });
        enc.encode(HeaderRepr {
            magic: MAGIC,
            format_version: FORMAT_VERSION,
            origin,
        })?;
        Ok(Self {
            enc,
            state: WriterState::Ready,
            sections: 0,
            events: 0,
        })
    }

    const fn ensure_ready(&self) -> Result<(), WriteError<W::Error>> {
        match self.state {
            WriterState::Ready => Ok(()),
            WriterState::Poisoned => Err(WriteError::Poisoned),
        }
    }

    /// Begin a section. The mutable borrow prevents interleaving sections.
    ///
    /// # Errors
    /// Returns [`WriteError`] on a poisoned writer, count overflow or sink error.
    pub fn section(
        &mut self,
        stream_id: &[u8],
    ) -> Result<SectionWriter<'_, W>, WriteError<W::Error>> {
        self.ensure_ready()?;
        self.state = WriterState::Poisoned;
        let next = self
            .sections
            .checked_add(1)
            .ok_or(WriteError::CountOverflow)?;
        self.enc.encode(HeadingRepr { stream_id })?;
        self.sections = next;
        self.state = WriterState::Ready;
        Ok(SectionWriter { writer: self })
    }

    /// Append completion evidence and recover the sink.
    ///
    /// Format 2 ends with tag 60000 and the definite array
    /// `[section_count, event_count, prefix_byte_count, prefix_crc32c]`.
    /// The prefix covers the header, every heading and every block. CRC32C
    /// detects accidental damage; it does not authenticate a backup.
    ///
    /// # Errors
    /// Returns [`WriteError`] if poisoned or if writing completion fails.
    pub fn finish(mut self) -> Result<W, WriteError<W::Error>> {
        self.ensure_ready()?;
        let prefix_bytes = self.enc.writer().bytes;
        let prefix_crc = self.enc.writer().crc;
        self.enc
            .tag(minicbor::data::Tag::new(COMPLETION_TAG))?
            .array(4)?
            .u64(self.sections)?
            .u64(self.events)?
            .u64(prefix_bytes)?
            .u32(prefix_crc)?;
        Ok(self.enc.into_writer().inner)
    }

    /// Recover unfinished bytes for explicit salvage or low-level inspection.
    /// This does not write completion evidence.
    pub fn into_unfinished_sink(self) -> W {
        self.enc.into_writer().inner
    }

    fn write_block(&mut self, event: &PersistedEnvelope) -> Result<(), WriteError<W::Error>> {
        let next = self
            .events
            .checked_add(1)
            .ok_or(WriteError::CountOverflow)?;
        let body = BodyRepr {
            version: event.version().as_u64(),
            schema_version: event.schema_version(),
            event_type: event.event_type(),
            metadata: event.metadata(),
            payload: event.payload(),
        };
        let body_bytes = minicbor::to_vec(&body).map_err(WriteError::Scratch)?;
        self.enc.encode(BlockRepr {
            crc: crc32c::crc32c(&body_bytes),
            body: &body_bytes,
        })?;
        self.events = next;
        Ok(())
    }
}

/// A section borrowed from a chunk writer. Blocks cannot precede a heading.
#[derive(Debug)]
pub struct SectionWriter<'a, W> {
    writer: &'a mut ChunkWriter<W>,
}

impl<W: minicbor::encode::Write> SectionWriter<'_, W> {
    /// Drain a fallible event stream. Failure or cancellation poisons the chunk.
    ///
    /// # Errors
    /// Returns [`SectionError::Read`] for source failures and
    /// [`SectionError::Write`] for write failures or an already poisoned writer.
    pub async fn try_extend<S, R>(
        &mut self,
        events: S,
    ) -> Result<&mut Self, SectionError<W::Error, R>>
    where
        S: Stream<Item = Result<PersistedEnvelope, R>>,
    {
        self.writer.ensure_ready()?;
        // Remain poisoned across every suspension. Only observing the source's
        // successful end restores readiness; dropping the future cannot do so.
        self.writer.state = WriterState::Poisoned;
        futures::pin_mut!(events);
        while let Some(item) = events.next().await {
            let env = item.map_err(SectionError::Read)?;
            self.writer.write_block(&env)?;
        }
        self.writer.state = WriterState::Ready;
        Ok(self)
    }

    /// Append one event with a CRC covering its body bytes.
    ///
    /// # Errors
    /// Returns [`WriteError`] on a poisoned writer, count overflow or sink error.
    pub fn block(&mut self, event: &PersistedEnvelope) -> Result<&mut Self, WriteError<W::Error>> {
        self.writer.ensure_ready()?;
        self.writer.state = WriterState::Poisoned;
        self.writer.write_block(event)?;
        self.writer.state = WriterState::Ready;
        Ok(self)
    }
}

/// Wire shape of a section heading map `{0: stream_id}`.
#[derive(minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
struct HeadingRepr<'a> {
    #[n(0)]
    #[cbor(with = "minicbor::bytes")]
    stream_id: &'a [u8],
}

/// Wire shape of a block body map. `global_seq` is deliberately absent —
/// store-local, restamped on import.
#[derive(minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
struct BodyRepr<'a> {
    #[n(0)]
    version: u64,
    #[n(1)]
    schema_version: u32,
    #[n(2)]
    event_type: &'a str,
    #[n(3)]
    #[cbor(with = "minicbor::bytes")]
    metadata: Option<&'a [u8]>,
    #[n(4)]
    #[cbor(with = "minicbor::bytes")]
    payload: &'a [u8],
}

/// Wire shape of a block array `[crc32c, body]`. Encode-only (decode is manual
/// so the crc is checked before the body is trusted).
#[derive(minicbor::Encode)]
#[cbor(array)]
struct BlockRepr<'a> {
    #[n(0)]
    crc: u32,
    #[n(1)]
    #[cbor(with = "minicbor::bytes")]
    body: &'a [u8],
}

/// Decode one block from the decoder's current position.
///
/// `Ok(Some(block))` is an event or corrupt marker; `Ok(None)` is a torn tail
/// for explicit salvage. Errors retain structural, parser and event domains.
fn decode_block(d: &mut Decoder<'_>) -> Result<Option<ImportBlock>, ChunkError> {
    match d.array() {
        Ok(Some(2)) => {}
        Ok(Some(_)) => {
            return Err(ChunkError::Malformed(
                "block array must have exactly 2 elements",
            ));
        }
        Ok(None) => return Err(ChunkError::Malformed("indefinite-length block array")),
        Err(e) if e.is_end_of_input() => return Ok(None),
        Err(source) => {
            return Err(ChunkError::Decode {
                context: "malformed block array",
                source,
            });
        }
    }
    let crc = match d.u32() {
        Ok(c) => c,
        Err(e) if e.is_end_of_input() => return Ok(None),
        Err(source) => {
            return Err(ChunkError::Decode {
                context: "malformed block crc",
                source,
            });
        }
    };
    let body = match d.bytes() {
        Ok(b) => b,
        Err(e) if e.is_end_of_input() => return Ok(None),
        Err(source) => {
            return Err(ChunkError::Decode {
                context: "malformed block body bytes",
                source,
            });
        }
    };
    if crc32c::crc32c(body) != crc {
        return Ok(Some(ImportBlock::Corrupt));
    }
    let mut body_decoder = Decoder::new(body);
    let parsed: BodyRepr = decode_map(&mut body_decoder).map_err(|source| ChunkError::Decode {
        context: "crc-valid body failed to decode",
        source,
    })?;
    if body_decoder.position() != body.len() {
        return Err(ChunkError::Malformed("trailing bytes in block body"));
    }
    Ok(Some(ImportBlock::Event(reconstruct(&parsed)?)))
}

/// Rebuild fields using checked offsets, rejecting invalid sizes before copying.
fn reconstruct(body: &BodyRepr<'_>) -> Result<PersistedEnvelope, BackupEventError> {
    use crate::value::{MAX_EVENT_TYPE_LEN, MAX_METADATA_LEN, MAX_PAYLOAD_LEN, ValueError};
    let version = Version::new(body.version).ok_or(BackupEventError::VersionZero)?;
    let schema = SchemaVersion::from_u32(body.schema_version)?;
    let event_type = body.event_type.as_bytes();
    if event_type.len() > MAX_EVENT_TYPE_LEN {
        return Err(ValueError::EventTypeTooLong {
            actual: event_type.len(),
        }
        .into());
    }
    if let Some(metadata) = body.metadata {
        if metadata.is_empty() {
            return Err(ValueError::MetadataEmpty.into());
        }
        if metadata.len() > MAX_METADATA_LEN {
            return Err(ValueError::MetadataTooLong {
                actual: metadata.len(),
            }
            .into());
        }
    }
    if body.payload.len() > MAX_PAYLOAD_LEN {
        return Err(ValueError::PayloadTooLong {
            actual: body.payload.len(),
        }
        .into());
    }
    let metadata_end = event_type
        .len()
        .checked_add(body.metadata.map_or(0, <[u8]>::len))
        .ok_or(BackupEventError::LengthOverflow)?;
    let total = metadata_end
        .checked_add(body.payload.len())
        .ok_or(BackupEventError::LengthOverflow)?;
    u32::try_from(total)?;
    let mut buf = Vec::with_capacity(total);
    buf.extend_from_slice(event_type);
    if let Some(metadata) = body.metadata {
        buf.extend_from_slice(metadata);
    }
    buf.extend_from_slice(body.payload);
    let compact = Bytes::from(buf);
    let event_kind = crate::value::EventType::from_bytes(compact.slice(..event_type.len()))?;
    let payload = crate::value::Payload::from_bytes(compact.slice(metadata_end..total))?;
    let metadata = body
        .metadata
        .map(|_| {
            crate::value::Metadata::from_bytes(
                compact.slice(event_kind.as_bytes().len()..metadata_end),
            )
        })
        .transpose()?;
    let frame = crate::wire::encode_frame(schema, &event_kind, &payload, metadata.as_ref())?;
    Ok(PersistedEnvelope::try_new(
        version,
        frame.value,
        schema,
        frame.offsets.event_type,
        frame.offsets.payload,
        frame.offsets.metadata,
    )?)
}

fn decode_completion(
    d: &mut Decoder<'_>,
    bytes: &[u8],
    prefix: usize,
    sections: u64,
    events: u64,
) -> Result<(), CompletionError> {
    let read = |source: minicbor::decode::Error| {
        if source.is_end_of_input() {
            CompletionError::Missing
        } else {
            CompletionError::Decode(source)
        }
    };
    if d.tag().map_err(read)? != minicbor::data::Tag::new(COMPLETION_TAG)
        || d.array().map_err(read)? != Some(4)
    {
        return Err(CompletionError::Shape);
    }
    let declared_sections = d.u64().map_err(read)?;
    let declared_events = d.u64().map_err(read)?;
    let declared = d.u64().map_err(read)?;
    let crc = d.u32().map_err(read)?;
    if d.position() != bytes.len() {
        return Err(CompletionError::TrailingBytes);
    }
    if declared_sections != sections || declared_events != events {
        return Err(CompletionError::Counts {
            declared_sections,
            sections,
            declared_events,
            events,
        });
    }
    let actual = u64::try_from(prefix).map_err(CompletionError::CountConversion)?;
    if declared != actual {
        return Err(CompletionError::Length { declared, actual });
    }
    if crc != crc32c::crc32c(&bytes[..prefix]) {
        return Err(CompletionError::Checksum);
    }
    Ok(())
}

/// Decode only complete, intact format-2 backups for normal restore.
/// Every error is returned before any destination store is accessed.
///
/// # Errors
/// Rejects legacy format 1, truncation, malformed framing, corrupt blocks,
/// mismatched completion counts/length/checksum and trailing bytes. For
/// intentional partial or legacy recovery use [`salvage_chunk`].
pub fn decode_chunk(bytes: &[u8]) -> Result<Vec<StreamSection>, ChunkError> {
    let recovered = salvage_chunk(bytes)?;
    recovered.completion?;
    if recovered
        .sections
        .iter()
        .flat_map(|s| &s.blocks)
        .any(|b| matches!(b, ImportBlock::Corrupt))
    {
        return Err(ChunkError::CorruptBlock);
    }
    Ok(recovered.sections)
}

/// Recover a prefix explicitly, retaining header and completion provenance.
///
/// A torn final item is omitted; CRC-failed blocks remain [`ImportBlock::Corrupt`].
/// No completeness claim is made for format 1, including apparently intact files.
///
/// # Errors
/// Returns [`ChunkError`] on unreadable headers or structural/event violations
/// other than a torn tail. Completion failures are retained in the result.
pub fn salvage_chunk(bytes: &[u8]) -> Result<SalvagedChunk, ChunkError> {
    let mut d = Decoder::new(bytes);
    let repr: HeaderRepr = decode_map(&mut d).map_err(|source| ChunkError::Decode {
        context: "unreadable header",
        source,
    })?;
    validate_header(repr.magic, repr.format_version)?;
    let header = ChunkHeader {
        format_version: repr.format_version,
        origin: repr.origin.map(Bytes::copy_from_slice),
    };
    let mut sections: Vec<StreamSection> = Vec::new();
    let mut event_count = 0_u64;
    let mut completion = Err(if header.format_version == 1 {
        CompletionError::Legacy
    } else {
        CompletionError::Missing
    });
    while d.position() < bytes.len() {
        let position = d.position();
        match d.datatype() {
            Ok(Type::Tag) if header.format_version == FORMAT_VERSION => {
                let section_count =
                    u64::try_from(sections.len()).map_err(CompletionError::CountConversion)?;
                completion = decode_completion(&mut d, bytes, position, section_count, event_count);
                break;
            }
            Ok(Type::Map) => match decode_map::<HeadingRepr>(&mut d) {
                Ok(heading) => sections.push(StreamSection {
                    origin: Bytes::copy_from_slice(heading.stream_id),
                    blocks: Vec::new(),
                }),
                Err(e) if e.is_end_of_input() => break,
                Err(source) => {
                    return Err(ChunkError::Decode {
                        context: "malformed section heading",
                        source,
                    });
                }
            },
            Ok(Type::Array) => {
                if sections.is_empty() {
                    return Err(ChunkError::Malformed("block before section heading"));
                }
                match decode_block(&mut d)? {
                    Some(block) => {
                        event_count = event_count
                            .checked_add(1)
                            .ok_or(CompletionError::CountOverflow)?;
                        if let Some(section) = sections.last_mut() {
                            section.blocks.push(block);
                        }
                    }
                    None => break,
                }
            }
            Ok(_) => return Err(ChunkError::Malformed("unexpected item type")),
            Err(e) if e.is_end_of_input() => break,
            Err(source) => {
                return Err(ChunkError::Decode {
                    context: "decode error",
                    source,
                });
            }
        }
    }
    Ok(SalvagedChunk {
        header,
        sections,
        completion,
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code asserts exact values"
)]
mod tests {
    use super::*;

    #[cfg(feature = "bytemuck")]
    #[test]
    fn decoded_backup_preserves_borrowed_codec_alignment() {
        use crate::Decode as _;
        use crate::codec::bytemuck::BytemuckCodec;
        let source = PersistedEnvelope::for_decode("E", &42_u128.to_ne_bytes()).unwrap();
        let codec = BytemuckCodec;
        let original: &u128 = codec.decode(&source).unwrap();
        assert_eq!(*original, 42);
        let mut writer = ChunkWriter::new(Vec::new(), None).unwrap();
        writer.section(b"stream").unwrap().block(&source).unwrap();
        let sections = decode_chunk(&writer.finish().unwrap()).unwrap();
        let ImportBlock::Event(restored) = &sections[0].blocks[0] else {
            panic!("intact block must decode");
        };
        let decoded: &u128 = codec.decode(restored).unwrap();
        assert_eq!(*decoded, 42);
        assert_eq!(
            restored
                .payload()
                .as_ptr()
                .align_offset(crate::wire::PAYLOAD_ALIGN),
            0
        );
    }

    fn with_completion(
        prefix: &[u8],
        sections: u64,
        events: u64,
        length: u64,
        crc: u32,
    ) -> Vec<u8> {
        let mut enc = Encoder::new(prefix.to_vec());
        enc.tag(minicbor::data::Tag::new(COMPLETION_TAG))
            .unwrap()
            .array(4)
            .unwrap()
            .u64(sections)
            .unwrap()
            .u64(events)
            .unwrap()
            .u64(length)
            .unwrap()
            .u32(crc)
            .unwrap();
        enc.into_writer()
    }

    #[test]
    fn completion_decoder_rejects_every_truncation_including_empty_backup() {
        for streams in [
            vec![],
            vec![
                (
                    b"a".as_slice(),
                    vec![persisted(1, 2, "Saved", Some(b"meta"), b"payload")],
                ),
                (
                    b"b".as_slice(),
                    vec![persisted(1, 1, "Saved", None, b"second")],
                ),
            ],
        ] {
            let bytes = encode_chunk(Some(b"producer"), &streams);
            assert_eq!(decode_chunk(&bytes).unwrap().len(), streams.len());
            for cut in 0..bytes.len() {
                assert!(
                    decode_chunk(&bytes[..cut]).is_err(),
                    "accepted cut {cut}/{}",
                    bytes.len()
                );
            }
        }
    }

    #[test]
    fn completion_decoder_checks_counts_length_checksum_and_trailing_bytes() {
        let mut writer = ChunkWriter::new(Vec::new(), None).unwrap();
        writer
            .section(b"a")
            .unwrap()
            .block(&persisted(1, 1, "E", None, b"first"))
            .unwrap();
        let prefix = writer.into_unfinished_sink();
        let length = u64::try_from(prefix.len()).unwrap();
        let crc = crc32c::crc32c(&prefix);
        for (sections, events) in [(0, 1), (2, 1), (1, 0), (1, 2), (u64::MAX, u64::MAX)] {
            assert!(matches!(
                decode_chunk(&with_completion(&prefix, sections, events, length, crc)),
                Err(ChunkError::Completion(CompletionError::Counts { .. }))
            ));
        }
        assert!(matches!(
            decode_chunk(&with_completion(&prefix, 1, 1, length + 1, crc)),
            Err(ChunkError::Completion(CompletionError::Length { .. }))
        ));
        assert!(matches!(
            decode_chunk(&with_completion(&prefix, 1, 1, length, crc ^ 1)),
            Err(ChunkError::Completion(CompletionError::Checksum))
        ));
        let complete = with_completion(&prefix, 1, 1, length, crc);
        assert_eq!(decode_chunk(&complete).unwrap().len(), 1);
        let mut extra = complete.clone();
        extra.push(0);
        assert!(matches!(
            decode_chunk(&extra),
            Err(ChunkError::Completion(CompletionError::TrailingBytes))
        ));
        extra = complete.clone();
        extra.extend_from_slice(&complete[prefix.len()..]);
        assert!(matches!(
            decode_chunk(&extra),
            Err(ChunkError::Completion(CompletionError::TrailingBytes))
        ));
        let mut changed = complete;
        let id = changed.iter().position(|byte| *byte == b'a').unwrap();
        changed[id] = b'b';
        assert!(matches!(
            decode_chunk(&changed),
            Err(ChunkError::Completion(CompletionError::Checksum))
        ));
    }

    #[test]
    fn completion_decoder_checks_record_shape_and_preserves_salvage_reason() {
        let prefix = header_bytes(None);
        for tail in [
            vec![0xd9, 0xea, 0x61, 0x84, 0, 0, 0, 0], // wrong tag
            vec![0xd9, 0xea, 0x60, 0x83, 0, 0, 0],    // wrong arity
            vec![0xd9, 0xea, 0x60, 0x9f, 0xff],
        ] {
            // indefinite array
            let bytes = [prefix.as_slice(), tail.as_slice()].concat();
            assert!(matches!(
                decode_chunk(&bytes),
                Err(ChunkError::Completion(CompletionError::Shape))
            ));
            assert!(matches!(
                salvage_chunk(&bytes).unwrap().completion(),
                Err(CompletionError::Shape)
            ));
        }
        let bytes = [prefix.as_slice(), &[0xd9, 0xea, 0x60, 0x84, 0x61, b'x']].concat();
        assert!(matches!(
            decode_chunk(&bytes),
            Err(ChunkError::Completion(CompletionError::Decode(_)))
        ));
    }

    #[test]
    fn completion_decoder_legacy_requires_explicit_recovery_with_provenance() {
        // Frozen format-1 header from the original golden snapshot, not a new
        // format-2 encoder relabeled complete. Block/heading layouts are unchanged.
        let mut legacy = vec![
            0xa3, 0x00, 0x44, 0x6e, 0x78, 0x63, 0x68, 0x01, 0x01, 0x02, 0x43, 0x64, 0x65, 0x76,
        ];
        legacy.extend_from_slice(&heading_bytes(b"a"));
        legacy.extend_from_slice(&block_bytes(&persisted(1, 1, "E", None, b"legacy")));
        assert!(matches!(
            decode_chunk(&legacy),
            Err(ChunkError::Completion(CompletionError::Legacy))
        ));
        let recovered = salvage_chunk(&legacy).unwrap();
        assert_eq!(recovered.header().format_version, 1);
        assert_eq!(
            recovered.header().origin.as_deref(),
            Some(b"dev".as_slice())
        );
        assert!(matches!(
            recovered.completion(),
            Err(CompletionError::Legacy)
        ));
        assert_eq!(recovered.sections()[0].origin.as_ref(), b"a");
        assert_eq!(recovered.into_sections_for_partial_recovery().len(), 1);
        let complete = encode_chunk(None, &[]);
        assert!(salvage_chunk(&complete).unwrap().completion().is_ok());
    }

    #[test]
    fn completion_decoder_rejects_duplicate_map_keys_and_body_trailing_bytes() {
        // A duplicate format key cannot be interpreted by last-key-wins decoding.
        let header = [0xa3, 0, 0x44, b'n', b'x', b'c', b'h', 1, 2, 1, 2];
        assert!(matches!(
            decode_chunk(&header),
            Err(ChunkError::Decode {
                context: "unreadable header",
                ..
            })
        ));
        let mut prefix = header_bytes(None);
        prefix.extend_from_slice(&[0xa2, 0, 0x41, b'a', 0, 0x41, b'b']);
        assert!(matches!(
            decode_chunk(&prefix),
            Err(ChunkError::Decode {
                context: "malformed section heading",
                ..
            })
        ));
        let mut body = minicbor::to_vec(BodyRepr {
            version: 1,
            schema_version: 1,
            event_type: "E",
            metadata: None,
            payload: b"data",
        })
        .unwrap();
        body.push(0);
        let block = minicbor::to_vec(BlockRepr {
            crc: crc32c::crc32c(&body),
            body: &body,
        })
        .unwrap();
        let mut decoder = Decoder::new(&block);
        assert!(matches!(
            decode_block(&mut decoder),
            Err(ChunkError::Malformed("trailing bytes in block body"))
        ));
    }

    #[test]
    fn completion_decoder_rejects_corrupt_blocks_with_valid_outer_completion() {
        let body = minicbor::to_vec(BodyRepr {
            version: 1,
            schema_version: 1,
            event_type: "E",
            metadata: None,
            payload: b"data",
        })
        .unwrap();
        let block = minicbor::to_vec(BlockRepr {
            crc: crc32c::crc32c(&body) ^ 1,
            body: &body,
        })
        .unwrap();
        let mut prefix = header_bytes(None);
        prefix.extend_from_slice(&heading_bytes(b"stream"));
        prefix.extend_from_slice(&block);
        let bytes = with_completion(
            &prefix,
            1,
            1,
            u64::try_from(prefix.len()).unwrap(),
            crc32c::crc32c(&prefix),
        );
        assert!(matches!(
            decode_chunk(&bytes),
            Err(ChunkError::CorruptBlock)
        ));
        let recovered = salvage_chunk(&bytes).unwrap();
        assert!(recovered.completion().is_ok());
        assert!(matches!(
            recovered.sections()[0].blocks[0],
            ImportBlock::Corrupt
        ));
    }

    #[test]
    fn completion_writer_footer_sink_failure_leaves_no_complete_artifact() {
        let full = ChunkWriter::new(Vec::new(), None)
            .unwrap()
            .finish()
            .unwrap();
        let mut storage = vec![0; full.len() - 1];
        {
            let result = ChunkWriter::new(storage.as_mut_slice(), None)
                .unwrap()
                .finish();
            assert!(matches!(result, Err(WriteError::Encode(error))
                if matches!(error.as_write(), Some(ChunkSinkError::Sink(_)))));
        }
        assert!(matches!(
            decode_chunk(&storage),
            Err(ChunkError::Completion(CompletionError::Missing))
        ));
    }

    #[test]
    fn completion_writer_records_exact_prefix_and_counts() {
        let mut writer = ChunkWriter::new(Vec::new(), Some(b"origin")).unwrap();
        writer.section(b"empty").unwrap();
        writer
            .section(b"events")
            .unwrap()
            .block(&persisted(1, 1, "E", Some(b"meta"), b"payload"))
            .unwrap();
        let prefix_len = writer.enc.writer().bytes;
        let prefix_crc = writer.enc.writer().crc;
        let bytes = writer.finish().unwrap();
        let offset = usize::try_from(prefix_len).unwrap();
        assert_eq!(crc32c::crc32c(&bytes[..offset]), prefix_crc);
        let mut decoder = Decoder::new(&bytes[offset..]);
        assert_eq!(
            decoder.tag().unwrap(),
            minicbor::data::Tag::new(COMPLETION_TAG)
        );
        assert_eq!(decoder.array().unwrap(), Some(4));
        assert_eq!(decoder.u64().unwrap(), 2);
        assert_eq!(decoder.u64().unwrap(), 1);
        assert_eq!(decoder.u64().unwrap(), prefix_len);
        assert_eq!(decoder.u32().unwrap(), prefix_crc);
        assert_eq!(decoder.position(), bytes.len() - offset);
    }

    #[test]
    fn completion_writer_source_failure_cannot_be_finished() {
        let mut writer = ChunkWriter::new(Vec::new(), None).unwrap();
        {
            let mut section = writer.section(b"events").unwrap();
            let source = futures::stream::iter([
                Ok(persisted(1, 1, "E", None, b"first")),
                Err(std::io::Error::from(std::io::ErrorKind::Interrupted)),
            ]);
            let error = futures::executor::block_on(section.try_extend(source)).unwrap_err();
            assert!(matches!(error, SectionError::Read(read_error)
                if read_error.kind() == std::io::ErrorKind::Interrupted));
            assert!(matches!(
                section.block(&persisted(2, 1, "E", None, b"second")),
                Err(WriteError::Poisoned)
            ));
        }
        assert!(matches!(
            writer.section(b"later"),
            Err(WriteError::Poisoned)
        ));
        assert!(matches!(writer.finish(), Err(WriteError::Poisoned)));
    }

    #[test]
    fn completion_writer_canceled_source_cannot_be_finished() {
        use futures::FutureExt as _;
        let mut writer = ChunkWriter::new(Vec::new(), None).unwrap();
        {
            let mut section = writer.section(b"events").unwrap();
            let source = futures::stream::iter([Ok::<_, std::io::Error>(persisted(
                1, 1, "E", None, b"first",
            ))])
            .chain(futures::stream::pending());
            assert!(section.try_extend(source).now_or_never().is_none());
        }
        assert_eq!(writer.events, 1);
        assert!(matches!(writer.finish(), Err(WriteError::Poisoned)));
    }

    #[test]
    fn completion_writer_sink_failure_cannot_be_finished() {
        // The actual minicbor slice sink runs out of space partway through a block.
        let mut storage = [0_u8; 32];
        let mut writer = ChunkWriter::new(storage.as_mut_slice(), None).unwrap();
        {
            let mut section = writer.section(b"events").unwrap();
            let error = section
                .block(&persisted(1, 1, "E", None, &[9; 128]))
                .unwrap_err();
            assert!(matches!(error, WriteError::Encode(encode_error)
                if matches!(encode_error.as_write(), Some(ChunkSinkError::Sink(_)))));
        }
        assert!(matches!(writer.finish(), Err(WriteError::Poisoned)));
    }

    #[test]
    fn completion_writer_count_and_length_overflow_fail_closed() {
        let mut section_writer = ChunkWriter::new(Vec::new(), None).unwrap();
        section_writer.sections = u64::MAX;
        assert!(matches!(
            section_writer.section(b"next"),
            Err(WriteError::CountOverflow)
        ));
        assert!(matches!(section_writer.finish(), Err(WriteError::Poisoned)));

        let mut event_writer = ChunkWriter::new(Vec::new(), None).unwrap();
        event_writer.section(b"events").unwrap();
        event_writer.events = u64::MAX;
        assert!(matches!(
            event_writer
                .section(b"next")
                .unwrap()
                .block(&persisted(1, 1, "E", None, b"x")),
            Err(WriteError::CountOverflow)
        ));
        assert!(matches!(event_writer.finish(), Err(WriteError::Poisoned)));

        let mut length_writer = ChunkWriter::new(Vec::new(), None).unwrap();
        length_writer.enc.writer_mut().bytes = u64::MAX;
        let error = length_writer.section(b"next").unwrap_err();
        assert!(matches!(error, WriteError::Encode(encode_error)
            if matches!(encode_error.as_write(), Some(ChunkSinkError::LengthOverflow))));
        assert!(matches!(length_writer.finish(), Err(WriteError::Poisoned)));
    }

    #[test]
    fn writer_multi_section_round_trips() {
        let mut w = ChunkWriter::new(Vec::new(), Some(b"dev")).expect("new");
        {
            let mut s = w.section(b"task-1").expect("section");
            s.block(&persisted(1, 1, "E", None, b"a1")).expect("block");
            s.block(&persisted(2, 1, "E", Some(b"m"), b"a2"))
                .expect("block");
        }
        {
            let mut s = w.section(b"task-2").expect("section");
            s.block(&persisted(1, 2, "E", None, b"b1")).expect("block");
        }
        let chunk = Bytes::from(w.finish().expect("finish"));

        let sections = decode_chunk(&chunk).expect("decode");
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].origin.as_ref(), b"task-1");
        assert_eq!(sections[0].blocks.len(), 2);
        assert_eq!(sections[1].origin.as_ref(), b"task-2");
        match (&sections[0].blocks[1], &sections[1].blocks[0]) {
            (ImportBlock::Event(a2), ImportBlock::Event(b1)) => {
                assert_eq!(a2.metadata(), Some(b"m".as_slice()));
                assert_eq!(a2.payload(), b"a2");
                assert_eq!(b1.schema_version(), 2);
                assert_eq!(b1.payload(), b"b1");
            }
            _ => panic!("expected Event blocks"),
        }
    }

    #[test]
    fn writer_empty_section_then_stream() {
        // A section with zero blocks, then a real one — the writer-path typestate
        // must stay sound across an empty section (heading with no following
        // blocks) and a subsequent section.
        let mut w = ChunkWriter::new(Vec::new(), None).expect("new");
        {
            let _s = w.section(b"empty").expect("section");
        }
        {
            let mut s = w.section(b"real").expect("section");
            s.block(&persisted(1, 1, "E", None, b"x")).expect("block");
        }
        let sections = decode_chunk(&Bytes::from(w.finish().expect("finish"))).expect("decode");
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].origin.as_ref(), b"empty");
        assert!(sections[0].blocks.is_empty());
        assert_eq!(sections[1].blocks.len(), 1);
    }

    #[test]
    fn writer_header_decodes() {
        let w = ChunkWriter::new(Vec::new(), Some(b"dev-1")).expect("new");
        let bytes = Bytes::from(w.finish().expect("finish"));
        let header = decode_header(&bytes).expect("decode header");
        assert_eq!(header.format_version, FORMAT_VERSION);
        assert_eq!(header.origin.as_deref(), Some(b"dev-1".as_slice()));
    }

    #[test]
    fn write_error_displays_and_is_error() {
        // Infallible sink error: the WriteError type must still be constructible and
        // implement std::error::Error so it composes in caller error chains.
        fn assert_error<E: std::error::Error>() {}
        assert_error::<WriteError<core::convert::Infallible>>();
        assert_error::<SectionError<core::convert::Infallible, std::io::Error>>();
    }

    /// Build a `PersistedEnvelope` directly: backing buffer = [`event_type` | metadata? | payload].
    fn persisted(
        version: u64,
        schema: u32,
        event_type: &str,
        metadata: Option<&[u8]>,
        payload: &[u8],
    ) -> PersistedEnvelope {
        let mut buf = Vec::new();
        buf.extend_from_slice(event_type.as_bytes());
        let et_end = u32::try_from(buf.len()).expect("fits");
        let meta_range = metadata.map(|m| {
            let start = u32::try_from(buf.len()).expect("fits");
            buf.extend_from_slice(m);
            start..u32::try_from(buf.len()).expect("fits")
        });
        let pl_start = u32::try_from(buf.len()).expect("fits");
        buf.extend_from_slice(payload);
        let pl_end = u32::try_from(buf.len()).expect("fits");
        PersistedEnvelope::try_new(
            Version::new(version).expect("nonzero"),
            Bytes::from(buf),
            SchemaVersion::from_u32(schema).expect("nonzero"),
            0..et_end,
            pl_start..pl_end,
            meta_range,
        )
        .expect("valid persisted")
    }

    /// Decode a single block from a freshly-encoded block buffer.
    fn decode_one_block(bytes: &[u8]) -> ImportBlock {
        let mut d = Decoder::new(bytes);
        decode_block(&mut d)
            .expect("not malformed")
            .expect("not torn")
    }

    /// Header bytes alone — the writer-built equivalent of the old `encode_header`.
    fn header_bytes(origin: Option<&[u8]>) -> Vec<u8> {
        ChunkWriter::new(Vec::new(), origin)
            .expect("header writer")
            .into_unfinished_sink()
    }

    /// Section-heading bytes alone: header+heading via the writer, minus the header
    /// prefix (a CBOR sequence is concatenation, so the tail is exactly the heading).
    fn heading_bytes(stream_id: &[u8]) -> Vec<u8> {
        let mut w = ChunkWriter::new(Vec::new(), None).expect("writer");
        w.section(stream_id).expect("section");
        let full = w.into_unfinished_sink();
        full[header_bytes(None).len()..].to_vec()
    }

    /// One block's bytes alone: header+heading+block via the writer, minus the
    /// header+heading prefix.
    fn block_bytes(event: &PersistedEnvelope) -> Vec<u8> {
        let mut w = ChunkWriter::new(Vec::new(), None).expect("writer");
        {
            let mut s = w.section(b"k").expect("section");
            s.block(event).expect("block");
        }
        let full = w.into_unfinished_sink();
        let prefix = {
            let mut w2 = ChunkWriter::new(Vec::new(), None).expect("writer");
            w2.section(b"k").expect("section");
            w2.into_unfinished_sink().len()
        };
        full[prefix..].to_vec()
    }

    #[test]
    fn block_round_trips_all_fields() {
        let event = persisted(7, 3, "AccountOpened", Some(b"hlc=42"), b"balance:100");
        let bytes = block_bytes(&event);
        match decode_one_block(&bytes) {
            ImportBlock::Event(got) => {
                assert_eq!(got.version().as_u64(), 7);
                assert_eq!(got.schema_version(), 3);
                assert_eq!(got.event_type(), "AccountOpened");
                assert_eq!(got.metadata(), Some(b"hlc=42".as_slice()));
                assert_eq!(got.payload(), b"balance:100");
            }
            ImportBlock::Corrupt => panic!("expected Event, got Corrupt"),
        }
    }

    #[test]
    fn block_round_trips_without_metadata_and_empty_payload() {
        let event = persisted(1, 1, "E", None, b"");
        let bytes = block_bytes(&event);
        match decode_one_block(&bytes) {
            ImportBlock::Event(got) => {
                assert_eq!(got.metadata(), None);
                assert_eq!(got.payload(), b"");
                assert_eq!(got.version().as_u64(), 1);
            }
            ImportBlock::Corrupt => panic!("expected Event"),
        }
    }

    #[test]
    fn block_with_flipped_body_byte_is_corrupt() {
        let event = persisted(2, 1, "E", None, b"hello");
        let mut v = block_bytes(&event);
        let last = v.len() - 1;
        v[last] ^= 0xFF;
        assert!(matches!(decode_one_block(&v), ImportBlock::Corrupt));
    }

    #[test]
    fn header_round_trips_without_origin() {
        let bytes = header_bytes(None);
        let header = decode_header(&bytes).expect("decode");
        assert_eq!(header.format_version, FORMAT_VERSION);
        assert_eq!(header.origin, None);
    }

    #[test]
    fn header_round_trips_with_origin() {
        let bytes = header_bytes(Some(b"phone-7"));
        let header = decode_header(&bytes).expect("decode");
        assert_eq!(header.format_version, FORMAT_VERSION);
        assert_eq!(header.origin.as_deref(), Some(b"phone-7".as_slice()));
    }

    #[test]
    fn header_rejects_bad_magic() {
        let mut v = header_bytes(None);
        let pos = v.iter().position(|&b| b == b'n').expect("magic present");
        v[pos] = b'X';
        let err = decode_header(&v).expect_err("bad magic rejected");
        assert!(matches!(err, ChunkError::Malformed("bad magic")));
    }

    #[test]
    fn header_rejects_truncated() {
        let bytes = header_bytes(Some(b"x"));
        let err = decode_header(&bytes[..bytes.len() / 2]).expect_err("truncated rejected");
        assert!(matches!(err, ChunkError::Decode { .. }));
    }

    /// Build a full multi-stream chunk through the public `ChunkWriter`.
    fn encode_chunk(origin: Option<&[u8]>, streams: &[(&[u8], Vec<PersistedEnvelope>)]) -> Bytes {
        let mut w = ChunkWriter::new(Vec::new(), origin).expect("writer");
        for (stream_id, events) in streams {
            let mut s = w.section(stream_id).expect("section");
            for e in events {
                s.block(e).expect("block");
            }
        }
        Bytes::from(w.finish().expect("finish"))
    }

    #[test]
    fn decode_chunk_round_trips_multi_stream() {
        let a = vec![
            persisted(1, 1, "E", None, b"a1"),
            persisted(2, 1, "E", Some(b"m"), b"a2"),
        ];
        let b = vec![persisted(1, 2, "E", None, b"b1")];
        let chunk = encode_chunk(
            Some(b"dev-1"),
            &[(b"task-1".as_slice(), a), (b"task-2".as_slice(), b)],
        );

        let sections = decode_chunk(&chunk).expect("decode");
        assert_eq!(sections.len(), 2);

        assert_eq!(sections[0].origin.as_ref(), b"task-1");
        assert_eq!(sections[0].blocks.len(), 2);
        match (&sections[0].blocks[0], &sections[0].blocks[1]) {
            (ImportBlock::Event(e1), ImportBlock::Event(e2)) => {
                assert_eq!(e1.version().as_u64(), 1);
                assert_eq!(e1.payload(), b"a1");
                assert_eq!(e2.metadata(), Some(b"m".as_slice()));
                assert_eq!(e2.payload(), b"a2");
            }
            _ => panic!("expected two Event blocks"),
        }

        assert_eq!(sections[1].origin.as_ref(), b"task-2");
        match &sections[1].blocks[0] {
            ImportBlock::Event(e) => {
                assert_eq!(e.schema_version(), 2);
                assert_eq!(e.payload(), b"b1");
            }
            ImportBlock::Corrupt => panic!("expected Event"),
        }
    }

    #[test]
    fn unfinished_header_is_recoverable_but_not_complete() {
        let chunk = header_bytes(None);
        assert!(matches!(
            decode_chunk(&chunk),
            Err(ChunkError::Completion(CompletionError::Missing))
        ));
        let recovered = salvage_chunk(&chunk).unwrap();
        assert!(recovered.sections().is_empty());
        assert!(matches!(
            recovered.completion(),
            Err(CompletionError::Missing)
        ));
    }

    #[test]
    fn decode_block_before_heading_is_malformed() {
        let mut chunk = header_bytes(None);
        let event = persisted(1, 1, "E", None, b"x");
        chunk.extend_from_slice(&block_bytes(&event));
        let err = decode_chunk(&chunk).expect_err("block before heading");
        assert!(matches!(
            err,
            ChunkError::Malformed("block before section heading")
        ));
    }

    #[test]
    fn decode_empty_section_then_stream() {
        let chunk = encode_chunk(
            None,
            &[
                (b"empty".as_slice(), vec![]),
                (b"real".as_slice(), vec![persisted(1, 1, "E", None, b"r")]),
            ],
        );
        let sections = decode_chunk(&chunk).expect("decode");
        assert_eq!(sections.len(), 2);
        assert!(sections[0].blocks.is_empty());
        assert_eq!(sections[0].origin.as_ref(), b"empty");
        assert_eq!(sections[1].blocks.len(), 1);
    }

    // ── Task 5: Defensive boundary — Corrupt vs Malformed ──────────────────

    #[test]
    fn flipped_body_byte_rejects_normal_restore_and_salvages_corrupt_block() {
        let chunk = encode_chunk(
            None,
            &[(b"s".as_slice(), vec![persisted(1, 1, "E", None, b"hello")])],
        );
        let mut v = chunk.to_vec();
        let payload = v.windows(5).position(|bytes| bytes == b"hello").unwrap();
        v[payload] ^= 0xFF;
        assert!(matches!(
            decode_chunk(&v),
            Err(ChunkError::Completion(CompletionError::Checksum))
        ));
        let sections = salvage_chunk(&v)
            .expect("framing intact")
            .into_sections_for_partial_recovery();
        assert_eq!(sections.len(), 1);
        assert!(matches!(sections[0].blocks[0], ImportBlock::Corrupt));
    }

    #[test]
    fn bad_magic_chunk_is_malformed() {
        let mut v = encode_chunk(
            None,
            &[(b"s".as_slice(), vec![persisted(1, 1, "E", None, b"x")])],
        )
        .to_vec();
        let pos = v.iter().position(|&b| b == b'n').expect("magic");
        v[pos] = b'Z';
        assert!(matches!(
            decode_chunk(&v),
            Err(ChunkError::Malformed("bad magic"))
        ));
    }

    #[test]
    fn unknown_format_version_is_malformed() {
        let repr = HeaderRepr {
            magic: MAGIC,
            format_version: 3,
            origin: None,
        };
        let bytes = minicbor::to_vec(&repr).expect("encode");
        assert!(matches!(
            decode_chunk(&bytes),
            Err(ChunkError::Malformed("unknown format version"))
        ));
    }

    #[test]
    fn unexpected_top_level_item_is_malformed() {
        let mut v = header_bytes(None);
        v.push(0x01); // CBOR uint 1 — neither map nor array
        assert!(matches!(
            decode_chunk(&v),
            Err(ChunkError::Malformed("unexpected item type"))
        ));
    }

    #[test]
    fn crc_valid_but_body_invalid_is_malformed() {
        // Craft a block whose body decodes to version 0 (illegal) with a
        // MATCHING crc → proves crc-pass-body-fail => Malformed (not Corrupt).
        let mut body = Vec::new();
        {
            let mut e = minicbor::Encoder::new(&mut body);
            e.map(4)
                .expect("map")
                .u32(0)
                .expect("k0")
                .u64(0)
                .expect("v0")
                .u32(1)
                .expect("k1")
                .u32(1)
                .expect("v1")
                .u32(2)
                .expect("k2")
                .str("E")
                .expect("v2")
                .u32(4)
                .expect("k4")
                .bytes(b"")
                .expect("v4");
        }
        let block = BlockRepr {
            crc: crc32c::crc32c(&body),
            body: &body,
        };
        let mut chunk = header_bytes(None);
        chunk.extend_from_slice(&heading_bytes(b"s"));
        chunk.extend_from_slice(&minicbor::to_vec(&block).expect("block"));
        assert!(matches!(
            decode_chunk(&chunk),
            Err(ChunkError::Event(BackupEventError::VersionZero))
        ));
    }

    // ── Task 6: Lifecycle — incremental append / truncation / valid prefix ──

    /// Byte length of a chunk holding the header, one heading, and the first `n`
    /// blocks — the valid-prefix cut after `n` blocks in the full chunk.
    fn prefix_len_after_n_blocks(
        stream_id: &[u8],
        events: &[PersistedEnvelope],
        n: usize,
    ) -> usize {
        let mut w = ChunkWriter::new(Vec::new(), None).expect("writer");
        {
            let mut s = w.section(stream_id).expect("section");
            for e in events.iter().take(n) {
                s.block(e).expect("block");
            }
        }
        w.into_unfinished_sink().len()
    }

    #[test]
    fn every_block_boundary_prefix_is_valid() {
        let events: Vec<_> = (1..=4).map(|v| persisted(v, 1, "E", None, b"p")).collect();
        let chunk = encode_chunk(None, &[(b"s".as_slice(), events.clone())]);
        for n in 0..=events.len() {
            let cut = prefix_len_after_n_blocks(b"s", &events, n);
            assert!(decode_chunk(&chunk[..cut]).is_err());
            let sections = salvage_chunk(&chunk[..cut])
                .expect("prefix valid")
                .into_sections_for_partial_recovery();
            let got = sections.first().map_or(0, |s| s.blocks.len());
            assert_eq!(got, n, "prefix after {n} blocks must decode to {n} blocks");
        }
    }

    #[test]
    fn torn_final_block_is_dropped_earlier_survive() {
        let events: Vec<_> = (1..=3)
            .map(|v| persisted(v, 1, "E", None, b"payload"))
            .collect();
        let cut = prefix_len_after_n_blocks(b"s", &events, 3) - 1;
        let chunk = encode_chunk(None, &[(b"s".as_slice(), events)]);
        assert!(decode_chunk(&chunk[..cut]).is_err());
        let sections = salvage_chunk(&chunk[..cut])
            .expect("valid prefix")
            .into_sections_for_partial_recovery();
        assert_eq!(sections[0].blocks.len(), 2, "torn 3rd block dropped");
        assert!(matches!(sections[0].blocks[0], ImportBlock::Event(_)));
    }

    #[test]
    fn empty_input_is_malformed_header() {
        assert!(matches!(
            decode_chunk(&[]),
            Err(ChunkError::Decode {
                context: "unreadable header",
                ..
            })
        ));
    }

    // ── Task 8: Golden-byte insta snapshots ─────────────────────────────────

    fn hex_of(bytes: &[u8]) -> String {
        bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn golden_header_bytes() {
        let bytes = header_bytes(Some(b"dev"));
        insta::assert_snapshot!("header_with_origin_hex", hex_of(&bytes));
    }

    #[test]
    fn golden_block_bytes() {
        let event = persisted(1, 1, "E", None, b"hi");
        let bytes = block_bytes(&event);
        insta::assert_snapshot!("block_v1_hex", hex_of(&bytes));
    }

    // ── Task 9: VOPR round-trip + crc single-byte-mutation property tests ───

    use proptest::prelude::*;

    /// (version, schema, `event_type`, metadata?, payload) — boundary-inclusive.
    fn event_strategy() -> impl Strategy<Value = (u64, u32, String, Option<Vec<u8>>, Vec<u8>)> {
        (
            prop_oneof![
                Just(1u64),
                Just(2),
                Just(u64::MAX - 1),
                Just(u64::MAX),
                1u64..1000
            ],
            prop_oneof![Just(1u32), Just(7), Just(u32::MAX), 1u32..100],
            // event_type spanning CBOR text-string length bands: inline (<24),
            // 1-byte length (24..=255), 2-byte length (256..).
            prop_oneof![
                Just(String::new()),
                Just("E".to_owned()),
                "[A-Za-z]{0,40}",
                Just("z".repeat(300)),
            ],
            // metadata: None, the 1-byte band, and the 2-byte band.
            prop_oneof![
                Just(None),
                proptest::collection::vec(any::<u8>(), 1..32).prop_map(Some),
                proptest::collection::vec(any::<u8>(), 256..400).prop_map(Some),
            ],
            // payload spanning inline / 1-byte / 2-byte length bands.
            prop_oneof![
                proptest::collection::vec(any::<u8>(), 0..64),
                proptest::collection::vec(any::<u8>(), 256..512),
            ],
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn vopr_chunk_round_trips(
            origin in prop_oneof![
                Just(None),
                proptest::collection::vec(any::<u8>(), 1..16).prop_map(Some)
            ],
            streams in proptest::collection::vec(
                (proptest::collection::vec(any::<u8>(), 0..12),
                 proptest::collection::vec(event_strategy(), 0..5)),
                0..4,
            ),
        ) {
            let built: Vec<(Vec<u8>, Vec<PersistedEnvelope>)> = streams
                .iter()
                .map(|(sid, evs)| {
                    let events = evs.iter().map(|(v, sc, et, md, pl)| {
                        persisted(*v, *sc, et, md.as_deref(), pl)
                    }).collect();
                    (sid.clone(), events)
                })
                .collect();

            let refs: Vec<(&[u8], Vec<PersistedEnvelope>)> =
                built.iter().map(|(s, e)| (s.as_slice(), e.clone())).collect();
            let chunk = encode_chunk(origin.as_deref(), &refs);
            let sections = decode_chunk(&chunk).expect("decode");

            prop_assert_eq!(sections.len(), built.len());
            for (section, (sid, events)) in sections.iter().zip(built.iter()) {
                prop_assert_eq!(section.origin.as_ref(), sid.as_slice());
                prop_assert_eq!(section.blocks.len(), events.len());
                for (block, original) in section.blocks.iter().zip(events.iter()) {
                    match block {
                        ImportBlock::Event(got) => {
                            prop_assert_eq!(got.version(), original.version());
                            prop_assert_eq!(got.schema_version(), original.schema_version());
                            prop_assert_eq!(got.event_type(), original.event_type());
                            prop_assert_eq!(got.metadata(), original.metadata());
                            prop_assert_eq!(got.payload(), original.payload());
                        }
                        ImportBlock::Corrupt => prop_assert!(false, "unexpected corrupt"),
                    }
                }
            }
        }

        #[test]
        fn vopr_single_byte_body_mutation_is_corrupt_never_silently_wrong(
            (ev_v, ev_sc, ev_et, ev_md, ev_pl) in event_strategy(),
            flip_pick in any::<prop::sample::Index>(),
        ) {
            let event = persisted(ev_v, ev_sc, &ev_et, ev_md.as_deref(), &ev_pl);
            let mut v = block_bytes(&event);
            let start = v.len() / 2;
            let idx = start + flip_pick.index(v.len() - start);
            v[idx] ^= 0xFF;
            let mut d = Decoder::new(&v);
            match decode_block(&mut d) {
                Ok(Some(ImportBlock::Event(got))) => {
                    prop_assert_eq!(got.payload(), event.payload());
                    prop_assert_eq!(got.event_type(), event.event_type());
                }
                Ok(Some(ImportBlock::Corrupt) | None) | Err(_) => {}
            }
        }
    }

    // ── Task 10: Defensive fuzz (never-panic) + forward-compat unknown key ──

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn decode_chunk_never_panics_on_arbitrary_bytes(
            bytes in proptest::collection::vec(any::<u8>(), 0..256),
        ) {
            let _ = decode_chunk(&bytes);
        }

        #[test]
        fn decode_chunk_never_panics_on_valid_header_plus_garbage(
            garbage in proptest::collection::vec(any::<u8>(), 0..128),
        ) {
            let mut v = header_bytes(None);
            v.extend_from_slice(&garbage);
            let _ = decode_chunk(&v);
        }
    }

    #[test]
    fn body_with_unknown_extra_key_still_decodes_to_event() {
        // Forward-compat: a future encoder adds key 5 without bumping the
        // format version; this decoder must skip it.
        let mut body = Vec::new();
        {
            let mut e = minicbor::Encoder::new(&mut body);
            e.map(5)
                .expect("map")
                .u32(0)
                .expect("k0")
                .u64(3)
                .expect("v0")
                .u32(1)
                .expect("k1")
                .u32(1)
                .expect("v1")
                .u32(2)
                .expect("k2")
                .str("E")
                .expect("v2")
                .u32(4)
                .expect("k4")
                .bytes(b"data")
                .expect("v4")
                .u32(5)
                .expect("k5")
                .u32(999)
                .expect("v5"); // unknown key
        }
        let block = BlockRepr {
            crc: crc32c::crc32c(&body),
            body: &body,
        };
        let mut chunk = header_bytes(None);
        chunk.extend_from_slice(&heading_bytes(b"s"));
        chunk.extend_from_slice(&minicbor::to_vec(&block).expect("block"));
        let sections = salvage_chunk(&chunk)
            .expect("decode")
            .into_sections_for_partial_recovery();
        match &sections[0].blocks[0] {
            ImportBlock::Event(e) => {
                assert_eq!(e.version().as_u64(), 3);
                assert_eq!(e.payload(), b"data");
            }
            ImportBlock::Corrupt => panic!("unknown key must be skipped, not corrupt"),
        }
    }

    // ── #2: CBOR length-form boundaries (deterministic) ──────────────────────
    // CBOR encodes a byte/text-string length with a different prefix per size
    // band: inline (<24), 1-byte (24..=255), 2-byte (256..=65535), 4-byte
    // (65536..). A bug mishandling one band is invisible until a value lands in
    // it; exercise each band explicitly through `block_bytes` → decode.

    #[test]
    fn box_round_trips_every_payload_length_band() {
        for size in [0usize, 23, 24, 255, 256, 1024, 65536] {
            let payload = vec![0xABu8; size];
            let event = persisted(1, 1, "E", None, &payload);
            let bytes = block_bytes(&event);
            match decode_one_block(&bytes) {
                ImportBlock::Event(got) => {
                    assert_eq!(got.payload().len(), size, "payload size {size} length");
                    assert_eq!(got.payload(), payload.as_slice(), "payload {size} bytes");
                }
                ImportBlock::Corrupt => panic!("payload size {size} must round-trip"),
            }
        }
    }

    #[test]
    fn box_round_trips_event_type_at_2byte_band_and_max() {
        for et_len in [256usize, crate::value::MAX_EVENT_TYPE_LEN] {
            let et = "a".repeat(et_len);
            let event = persisted(1, 1, &et, Some(b"m"), b"p");
            let bytes = block_bytes(&event);
            match decode_one_block(&bytes) {
                ImportBlock::Event(got) => {
                    assert_eq!(got.event_type().len(), et_len, "event_type len {et_len}");
                    assert_eq!(got.event_type(), et);
                    assert_eq!(got.metadata(), Some(b"m".as_slice()));
                }
                ImportBlock::Corrupt => panic!("event_type len {et_len} must round-trip"),
            }
        }
    }

    #[test]
    fn box_round_trips_metadata_at_2byte_band() {
        let meta = vec![0x07u8; 300];
        let event = persisted(1, 1, "E", Some(&meta), b"p");
        match decode_one_block(&block_bytes(&event)) {
            ImportBlock::Event(got) => assert_eq!(got.metadata(), Some(meta.as_slice())),
            ImportBlock::Corrupt => panic!("metadata 2-byte band must round-trip"),
        }
    }

    // ── #3: defensive decode branches, asserted by specific outcome ──────────
    // These branches were previously exercised only by the never-panic fuzz,
    // which asserts no crash but not WHICH result. Pin the exact outcomes.

    #[test]
    fn decode_block_rejects_indefinite_array() {
        // 0x9f = indefinite-length array. Never emitted; must be rejected, not
        // hang or panic. (Reached via decode_block directly; decode_chunk peeks
        // ArrayIndef as a distinct type and rejects it before calling here.)
        let mut d = Decoder::new(&[0x9fu8]);
        assert!(matches!(
            decode_block(&mut d),
            Err(ChunkError::Malformed("indefinite-length block array"))
        ));
    }

    #[test]
    fn decode_block_rejects_wrong_arity() {
        // A 3-element array violates the exactly-2 block shape.
        let mut buf = Vec::new();
        {
            let mut e = minicbor::Encoder::new(&mut buf);
            e.array(3).expect("array header");
        }
        let mut d = Decoder::new(&buf);
        assert!(matches!(
            decode_block(&mut d),
            Err(ChunkError::Malformed(
                "block array must have exactly 2 elements"
            ))
        ));
    }

    #[test]
    fn indefinite_array_at_top_level_is_malformed() {
        // decode_chunk peeks Type::ArrayIndef (not Array) → unexpected item type.
        let mut chunk = header_bytes(None);
        chunk.extend_from_slice(&heading_bytes(b"s"));
        chunk.push(0x9f); // array(*) indefinite
        chunk.push(0xff); // break
        assert!(matches!(
            decode_chunk(&chunk),
            Err(ChunkError::Malformed("unexpected item type"))
        ));
    }

    #[test]
    fn torn_mid_heading_stops_at_valid_prefix() {
        // One complete section, then a heading truncated mid stream-id. The
        // complete section survives; the torn heading yields no section (it is a
        // valid-prefix stop, not Malformed).
        let chunk = encode_chunk(
            None,
            &[(b"done".as_slice(), vec![persisted(1, 1, "E", None, b"x")])],
        );
        let prefix = prefix_len_after_n_blocks(b"done", &[persisted(1, 1, "E", None, b"x")], 1);
        let mut v = chunk[..prefix].to_vec();
        let heading = heading_bytes(b"truncated-stream-id");
        v.extend_from_slice(&heading[..heading.len() - 4]); // drop 4 id bytes
        let sections = salvage_chunk(&v)
            .expect("valid prefix")
            .into_sections_for_partial_recovery();
        assert_eq!(sections.len(), 1, "torn heading produces no section");
        assert_eq!(sections[0].origin.as_ref(), b"done");
        assert_eq!(sections[0].blocks.len(), 1);
    }

    #[test]
    fn header_missing_magic_key_is_malformed() {
        // A 1-entry map carrying only format_version (key 1), no magic (key 0):
        // a missing required field, distinct from a corrupted magic value.
        let mut bytes = Vec::new();
        {
            let mut e = minicbor::Encoder::new(&mut bytes);
            e.map(1)
                .expect("map")
                .u32(1)
                .expect("k1")
                .u32(1)
                .expect("v1");
        }
        assert!(matches!(
            decode_header(&bytes),
            Err(ChunkError::Decode {
                context: "unreadable header",
                ..
            })
        ));
        assert!(matches!(
            decode_chunk(&bytes),
            Err(ChunkError::Decode {
                context: "unreadable header",
                ..
            })
        ));
    }

    // ── Task 4: try_extend — drains a fallible stream into a section ──────────

    #[tokio::test]
    async fn try_extend_drains_stream_into_section() {
        let events = vec![
            Ok::<_, std::io::Error>(persisted(1, 1, "E", None, b"x1")),
            Ok(persisted(2, 1, "E", Some(b"m"), b"x2")),
        ];
        let mut w = ChunkWriter::new(Vec::new(), None).expect("new");
        w.section(b"s")
            .expect("section")
            .try_extend(futures::stream::iter(events))
            .await
            .expect("extend");
        let chunk = Bytes::from(w.finish().expect("finish"));

        let sections = decode_chunk(&chunk).expect("decode");
        assert_eq!(sections[0].blocks.len(), 2);
        match &sections[0].blocks[1] {
            ImportBlock::Event(e) => assert_eq!(e.payload(), b"x2"),
            ImportBlock::Corrupt => panic!("expected Event"),
        }
    }

    #[tokio::test]
    async fn try_extend_surfaces_read_error_distinctly() {
        let boom = std::io::Error::other("boom");
        let events = vec![Ok(persisted(1, 1, "E", None, b"x1")), Err(boom)];
        let mut w = ChunkWriter::new(Vec::new(), None).expect("new");
        let err = w
            .section(b"s")
            .expect("section")
            .try_extend(futures::stream::iter(events))
            .await
            .expect_err("read error propagates");
        match err {
            SectionError::Read(io) => assert_eq!(io.to_string(), "boom"),
            SectionError::Write(_) => panic!("a read failure must not be a write failure"),
        }
    }

    #[test]
    fn crc_valid_body_with_empty_metadata_is_malformed() {
        // metadata present but empty violates the Metadata non-empty invariant;
        // reconstruct → try_new rejects the empty range → Malformed (with a
        // matching crc, proving the rejection is at reconstruction, not crc).
        let mut body = Vec::new();
        {
            let mut e = minicbor::Encoder::new(&mut body);
            e.map(5)
                .expect("map")
                .u32(0)
                .expect("k0")
                .u64(1)
                .expect("v0")
                .u32(1)
                .expect("k1")
                .u32(1)
                .expect("v1")
                .u32(2)
                .expect("k2")
                .str("E")
                .expect("v2")
                .u32(3)
                .expect("k3")
                .bytes(b"")
                .expect("v3") // empty metadata
                .u32(4)
                .expect("k4")
                .bytes(b"p")
                .expect("v4");
        }
        let block = BlockRepr {
            crc: crc32c::crc32c(&body),
            body: &body,
        };
        let mut chunk = header_bytes(None);
        chunk.extend_from_slice(&heading_bytes(b"s"));
        chunk.extend_from_slice(&minicbor::to_vec(&block).expect("block"));
        assert!(matches!(
            decode_chunk(&chunk),
            Err(ChunkError::Event(BackupEventError::Value(
                crate::value::ValueError::MetadataEmpty
            )))
        ));
    }
}
