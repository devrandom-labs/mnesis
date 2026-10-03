# CBOR backup format and recovery

The writer produces format 2. A complete artifact consists of a header, zero
or more stream sections, and exactly one completion record at the end. The
application adds completion evidence because a CBOR sequence alone cannot
detect entirely missing items ([RFC 8742, section 2](https://www.rfc-editor.org/rfc/rfc8742.html#section-2)).

The following CDDL describes the application records. Integer keys are
unsigned; maps and arrays have definite lengths. Duplicate map keys reject.
Unknown unsigned 32-bit numeric map fields are skipped for forward compatibility. Required
fields remain mandatory.

```cddl
header = {0: bstr, 1: uint, ? 2: bstr / null}
heading = {0: bstr}
block = [uint, bstr]
body = {0: uint, 1: uint, 2: tstr, ? 3: bstr / null, 4: bstr}
completion = #6.60000([uint, uint, uint, uint])
```

Header key 0 is `h'6e786368'` (`nxch`), key 1 is the format version, and key 2
is optional producer identity. A heading carries the exact stream ID bytes.
Each following block belongs to that heading until the next heading or
completion. A block is `[crc32c(body_bytes), body_bytes]`. The body keys are
event version, schema version, event type, optional metadata, and payload.
Null optional fields decode as absent; the writer omits absent fields.
Event/schema versions must be nonzero; metadata, when present as bytes, must be
nonempty. Envelope size limits apply before reconstruction copies data.
Global positions are intentionally absent and are reassigned on import.

Tag 60000 is this application's completion marker, not a CBOR-sequence
terminator defined by RFC 8742. Its array contains, in order:

1. Section count, including empty sections.
2. Event-block count, including any corrupt markers encountered during salvage.
3. Byte length of the entire prefix before the completion tag.
4. CRC32C of those exact prefix bytes, including the header, all headings and
   all block framing and contents.

Counts and length are unsigned 64-bit values; CRCs are unsigned 32-bit values.
Decoding checks the actual counts, length, checksum and end of input. Extra
bytes or a second completion record reject. CRC32C detects accidental damage;
it does not authenticate hostile edits or identify an authorized producer.

## Complete export and normal restore

List and read streams from one `ExportSession` so the source view is consistent.
Drain every intended stream successfully, close the session, then call
`ChunkWriter::finish()`. Only `finish` emits completion evidence. A source
error, canceled drain, sink failure or count overflow poisons the writer and
prevents a later finish. `into_unfinished_sink()` explicitly recovers a prefix
without completing it. Finishing does not flush or sync the underlying sink;
the caller applies its file durability/publication policy.

Pass the whole artifact through `decode_chunk` before calling the importer.
Normal decoding rejects every truncated format-2 artifact, legacy format 1,
invalid event fields, corrupt blocks, bad completion evidence and trailing
bytes. No destination is accessed during decoding. The importer retains its
existing request contract: `WholeChunk` applies the supplied request atomically;
`PerStream` may retain earlier completed runs after a later storage failure.
Completion proves that the artifact matches the writer's finished content;
it cannot prove that the producer selected every intended source stream.

## Explicit salvage and legacy migration

`salvage_chunk` returns a distinct `SalvagedChunk`: the original header,
recovered sections and a completion result. A torn final item is omitted;
CRC-failed blocks remain corrupt markers. Structural or invalid-field errors
still reject. Completion damage is retained as its typed error, and a valid
completion record can coexist with a deliberately invalid per-block checksum.
Inspect blocks as well as completion status before recovery.

Format 1 has no completion record. Even an apparently intact legacy file
always reports `CompletionError::Legacy`; absence of truncation cannot be
established from its bytes. To recover or migrate it:

1. Preserve the original artifact and its producer identity.
2. Inspect `salvage_chunk` and reconcile recovered stream IDs, event versions
   and contents against independent source records where available. Keep
   uncertainty explicit when those records are unavailable.
3. Deliberately call `into_sections_for_partial_recovery()` and import into a
   separate destination using the chosen import policy. Keep the import report.
4. Export the reconciled destination through a new session and finish a format-2
   artifact. This establishes completeness of that new export, without proving
   completeness of the historical source. Keep the original provenance with it.

Both decoders allocate for data actually present in the supplied byte slice,
not declared CBOR collection lengths. They materialize recovered sections and
events; they are not constant-memory streaming restore APIs. Field offsets,
combined sizes, byte lengths and counts use checked arithmetic.
