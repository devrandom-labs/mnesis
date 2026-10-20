# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.3](https://github.com/devrandom-labs/mnesis/compare/mnesis-postgres-v0.5.2...mnesis-postgres-v0.5.3) - 2026-10-20

### Other

- *(postgres)* complete capacity and acceptance evidence ([#407](https://github.com/devrandom-labs/mnesis/pull/407))
- *(postgres)* measure sustained work and operator recovery ([#405](https://github.com/devrandom-labs/mnesis/pull/405))

## [0.5.2](https://github.com/devrandom-labs/mnesis/compare/mnesis-postgres-v0.5.1...mnesis-postgres-v0.5.2) - 2026-10-19

### Added

- *(postgres)* trace operational progress and transaction outcomes ([#402](https://github.com/devrandom-labs/mnesis/pull/402))

- Optional tracing for listener recovery, bounded scans, fallback rescans and
  transaction outcomes, with fixed labels and argument-free operation spans.

## [0.5.0](https://github.com/devrandom-labs/mnesis/compare/mnesis-postgres-v0.4.0...mnesis-postgres-v0.5.0) - 2026-10-19

### Added

- *(postgres)* enforce the local durable write profile ([#395](https://github.com/devrandom-labs/mnesis/pull/395))
- *(postgres)* enable verified TLS and prove runtime permissions ([#392](https://github.com/devrandom-labs/mnesis/pull/392))
- *(postgres)* [**breaking**] bind resume positions to their event history ([#387](https://github.com/devrandom-labs/mnesis/pull/387))
- *(postgres)* retain finite consistent export views ([#385](https://github.com/devrandom-labs/mnesis/pull/385))
- *(postgres)* enumerate committed event streams ([#384](https://github.com/devrandom-labs/mnesis/pull/384))
- *(postgres)* persist projection checkpoints ([#383](https://github.com/devrandom-labs/mnesis/pull/383))
- *(postgres)* persist aggregate snapshots ([#382](https://github.com/devrandom-labs/mnesis/pull/382))
- *(postgres)* commit atomic imports across streams ([#381](https://github.com/devrandom-labs/mnesis/pull/381))
- *(postgres)* add explicit validated schema migrations ([#379](https://github.com/devrandom-labs/mnesis/pull/379))

### Fixed

- *(postgres)* bound runtime migration metadata reads ([#398](https://github.com/devrandom-labs/mnesis/pull/398))
- *(postgres)* use native lengths for row limits ([#396](https://github.com/devrandom-labs/mnesis/pull/396))
- *(postgres)* preserve stream identities across caller query names ([#394](https://github.com/devrandom-labs/mnesis/pull/394))
- *(postgres)* preserve global ordering across caller search paths ([#388](https://github.com/devrandom-labs/mnesis/pull/388))
- *(postgres)* preserve full event values and binary identities ([#380](https://github.com/devrandom-labs/mnesis/pull/380))
- *(postgres)* preserve uncertain commit outcomes ([#377](https://github.com/devrandom-labs/mnesis/pull/377))
- *(postgres)* bound replay and recover subscriptions ([#376](https://github.com/devrandom-labs/mnesis/pull/376))

### Other

- *(postgres)* verify backup routing and recovery failures ([#391](https://github.com/devrandom-labs/mnesis/pull/391))
- *(postgres)* verify physical backup and consumer recovery ([#390](https://github.com/devrandom-labs/mnesis/pull/390))
- *(postgres)* verify bounded backups and projection recovery ([#389](https://github.com/devrandom-labs/mnesis/pull/389))

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
