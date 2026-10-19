# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Native pool, statement and lock deadline coverage, including startup,
  rollback of staged inserts, and shutdown during permanent server unavailability.
- Native durability checks before opening, migration and write acknowledgement,
  preserving configured commit modes and uncertain transaction finalization.
- Finite consistent export sessions with one native view across listing and
  stream reads, bounded pages, and cleanup on expiry, Drop, or caller pool close.
- Bounded event-only stream listing with frozen committed visibility, binary IDs,
  released pool connections, and native allocation/visibility regressions.
- Projection checkpoints with revision CAS, atomic state/position replacement,
  explicit rebuild, bounded reads, and preserved commit uncertainty.
- Migration4 and native coverage for projection consumers, concurrent writers,
  cancellation, client termination, restricted roles, and upgrade repair.

### Fixed

- Bound runtime migration metadata reads while preserving exact checksum and
  unsupported-version rejection.
- Application `octet_length` functions cannot bypass event, stream-ID, snapshot
  or checkpoint row byte limits.

## [0.4.0](https://github.com/devrandom-labs/mnesis/compare/mnesis-postgres-v0.3.1...mnesis-postgres-v0.4.0) - 2026-10-03

### Fixed

- [**breaking**] harden non-postgres contracts and retain audit regressions ([#373](https://github.com/devrandom-labs/mnesis/pull/373))

## [0.2.2](https://github.com/devrandom-labs/mnesis/compare/mnesis-postgres-v0.2.1...mnesis-postgres-v0.2.2) - 2026-08-13

### Other

- release v0.2.1 ([#360](https://github.com/devrandom-labs/mnesis/pull/360))

## [0.2.1](https://github.com/devrandom-labs/mnesis/compare/mnesis-postgres-v0.2.0...mnesis-postgres-v0.2.1) - 2026-08-13

### Other

- release v0.2.1 ([#357](https://github.com/devrandom-labs/mnesis/pull/357))

## [0.2.0](https://github.com/devrandom-labs/mnesis/compare/mnesis-postgres-v0.1.0...mnesis-postgres-v0.2.0) - 2026-07-28

### Added

- *(store)* [**breaking**] read-your-writes — append/repository surface the $all position ([#330](https://github.com/devrandom-labs/mnesis/pull/330)) ([#343](https://github.com/devrandom-labs/mnesis/pull/343))
- *(store)* [**breaking**] $all items carry their StreamKey ([#333](https://github.com/devrandom-labs/mnesis/pull/333)) ([#336](https://github.com/devrandom-labs/mnesis/pull/336))

### Fixed

- *(store)* audit hardening across store + adapters ([#347](https://github.com/devrandom-labs/mnesis/pull/347))

### Other

- *(store)* centralize the append-version contract in the kernel ([#337](https://github.com/devrandom-labs/mnesis/pull/337))
