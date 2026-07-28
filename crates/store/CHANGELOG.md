# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/devrandom-labs/mnesis/compare/mnesis-store-v0.1.0...mnesis-store-v0.2.0) - 2026-07-28

### Added

- *(store)* builder-level metadata provider on the typed facade ([#344](https://github.com/devrandom-labs/mnesis/pull/344)) ([#354](https://github.com/devrandom-labs/mnesis/pull/354))
- *(store)* attributed projector fold — apply_attributed carries the $all StreamKey ([#345](https://github.com/devrandom-labs/mnesis/pull/345)) ([#353](https://github.com/devrandom-labs/mnesis/pull/353))
- *(store)* [**breaking**] read-your-writes — append/repository surface the $all position ([#330](https://github.com/devrandom-labs/mnesis/pull/330)) ([#343](https://github.com/devrandom-labs/mnesis/pull/343))
- *(kernel)* [**breaking**] Handle decides Option<Events> — a no-op is a decision ([#329](https://github.com/devrandom-labs/mnesis/pull/329)) ([#340](https://github.com/devrandom-labs/mnesis/pull/340))
- *(store)* [**breaking**] $all items carry their StreamKey ([#333](https://github.com/devrandom-labs/mnesis/pull/333)) ([#336](https://github.com/devrandom-labs/mnesis/pull/336))
- *(store)* generalize Projection stepper + PersistTrigger over $all positions ([#335](https://github.com/devrandom-labs/mnesis/pull/335))

### Fixed

- *(store)* audit hardening across store + adapters ([#347](https://github.com/devrandom-labs/mnesis/pull/347))

### Other

- *(store)* mutation gate for mnesis-store ([#352](https://github.com/devrandom-labs/mnesis/pull/352))
- Add CodSpeed continuous performance measurement ([#339](https://github.com/devrandom-labs/mnesis/pull/339))
- *(store)* centralize the append-version contract in the kernel ([#337](https://github.com/devrandom-labs/mnesis/pull/337))
- consolidate duplicated toy Counter domain into a shared crate ([#239](https://github.com/devrandom-labs/mnesis/pull/239)) ([#320](https://github.com/devrandom-labs/mnesis/pull/320))
