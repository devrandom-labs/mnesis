# Dead ends

No production experiment has been rejected or reverted. Initial read/wake
experiments passed and are committed; later feature acceptance remains open.

Do not use `PgPool::size() == 0` as proof that listener setup ended. SQLx counts
in-flight connection attempts, and this assertion failed in the lifecycle
fixture despite no established LISTEN session. The retained fixture observes
Postgres LISTEN backends and the configured acquisition deadline instead.

- PG1 byte-edge test initially attempted present-but-empty metadata. The kernel's
  `Metadata` constructor rejects it (`MetadataEmpty`), so this is not an accepted
  envelope state. The retained test covers absent versus one-byte metadata;
  stored invalid metadata remains an error rather than weakening that contract.
