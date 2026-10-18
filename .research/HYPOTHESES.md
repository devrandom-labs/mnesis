# Hypotheses

The first H1/H2 read and wake experiments pass native regressions and the full
Darwin gate; their remaining feature acceptance is open. H3–H5 are proposed.
Their falsifiers and rollback
boundaries are in the [PRD hypothesis table](../docs/prds/postgres.md#hypotheses-and-falsifiers).
Associated workloads and observable laws are in PG0–PG8. Start with real-database
reproduction, not production refactoring. Record each experiment before editing.

Fresh audit follow-ups: PG4 must measure incompressible long binary keys against
Postgres's B-tree tuple limit; the 5,000-byte notification witness is compressible
and does not establish arbitrary-key capacity. Record practical accepted limits
or a native representation that preserves equality without probabilistic identity.
Do not silently substitute a hash for the durable stream identity. PG9 must
assess compatible patched event-listener >=5.4.2 rather than ignore the advisory.

Native PG4 probe now rejects a5,120-byte incompressible key in the legacy B-tree,
while EXCLUDE USING hash(key WITH =) accepts it and rejects duplicates. Actual
hash-colliding distinct keys survive equality recheck. This supported native
constraint supersedes a handcrafted advisory-lock identity protocol as the
preferred candidate; full adapter/migration/concurrency fitness still untested.

PG8: a transaction in another database on the same Postgres cluster can pin the
safe $all watermark. Native22861→22862 probe demonstrates this. Require an actual
adapter subscription recovery witness after a foreign-DB transaction ends with
no later writes, and document cluster-level long-transaction limits/diagnostics.
Never run independent native suites concurrently on one cluster; database-only
fixture separation does not isolate snapshot xmin. No unsafe shortcut authorized.
PG5 ordering hypothesis: validate ALL target heads using reads before ANY
transaction write can allocate its xid8. A new-key/catalog write or the first
run's event insert before reading another target could give its later version a
lower txid than its predecessor committed concurrently. Require a native schedule
that can reverse per-stream global delivery under such an interleaving; no custom
identity/fencing protocol. Resolve native key IDs only after all heads validate.

PG7 owner contract verified in crates/store/src/export.rs: ExportSession exposes
listing and per-stream reads, deliberately no global-read/write capability.
Use one repeatable MVCC view for complete visible histories; do not apply the
ordinary $all safety cutoff and omit visible rows. Do not implement RawEventStore
on the session merely to reuse code. Borrowed cursors, independent expiry and
explicit waited cleanup remain required. Investigate native PostgreSQL17
transaction_timeout plus existing SQLx connection ownership before custom machinery.

PG5 next experiment (after retained PG4 representations): existing AtomicAppend
under import feature, no new public method/error/runtime/dep. Budget <=+210/-60/net170
production Rust, <=+420 native test lines and <=20 manifest/feature registration
lines. Use a private child module of store so internal SQL owners stay private.
Validate distinct targets, all run versions and ALL row sizes before BEGIN; then
read/validate ALL committed heads before ANY key or event write assigns xid8.
Only then resolve missing key IDs and insert every run in one transaction. Shared
single-row persistence returns PostgresError; append/atomic boundaries separately
map only the exact optimistic constraint into their existing conflict metadata.
Keep server rejection/unknown commit mapping and exact sqlx source unchanged.
Return highest position, empty input performs no storage. Whole-batch local wakes
must precede one bounded native hint deadline so N targets cannot multiply the
post-commit auxiliary wait. No actor/transaction framework or controller IDs.
Falsifiers: malformed final run/route and oversize tail on a closed pool reject
before storage; stale final head/no writes; injected later SQL fault rolls back
all keys/events; actual native race rollback; real backend COMMIT loss leaves
all runs durable/unknown; head/XID interleaving cannot deliver a stream successor
before its predecessor; import WholeChunk/PerStream and ordinary subscribers see
only complete committed transactions. Register shared atomic conformance plus
owner-local faults; every run gets three reviews. Native SSI/deadlock evidence
and final import checkpoint resume laws remain PG3/PG5 obligations.

PG9 Nix-input hypothesis: current SQL source inclusion is an unscoped suffix
filter. Adding a docs-only SQL query-plan artifact caused the normal commit hook
to rebuild all12 checks although tested adapter source bytes are unchanged.
Falsifier: derivation identity remains unchanged after a docs SQL edit. Compare
supported cleanSourceWith filtering scoped to crate-owned SQL before retaining
any Nix change; do not bypass the current hook/gate. No custom validation tool.

PG5B native schedules before closing PG5: actual final-run unique collision must
roll back an earlier staged run and report a freshly observed actual head. Revoke
only keys SELECT from a dedicated native runtime role while its INSERT is waiting,
then release the winner: failed reconciliation must retain the ORIGINAL23505
Store source, never invent a fresh/unknown Conflict. No extra proxy framework.
Demonstrate all-heads-before-XID ordering with a blocked second-head query and a
concurrent predecessor; control must either reject the stale batch or preserve
per-stream global order. Add actualSSI/deadlock observations, and test existing
WholeChunk/PerStream import composition. Small separate test budgets per retained
experiment; no capability acceptance is dropped to fit the first slice budget.

PG5B1 ordering proof budget BEFORE test change: <=65 lines extending the EXISTING
private Unix forwarder to hold a Bind carrying one distinctive raw second-head
key, plus <=100 native test lines; no production changes/framework. Startup and
framed frontend traffic use existing Tokio I/O, only this selected mode frames
frontend packets. Hold BEFORE Bind reaches server (inside/after SELECT is too
late because its snapshot already exists). While held, controller commits B1;
atomic request expects B1 and writes A1+B2. Correct algorithm has no xid yet and
all delivery shows B1 before B2; early A-write mutant obtains earlier xid and
would show B2 before B1. Require actual observed hold, independent backend_xid,
real controller commit, exact full delivery and returned-position checks. Keep
mode private to tests and default relay unchanged. Kill an archived early-write
mutation before retaining this verification pass.

PG5B2 race/reconciliation pass budget: <=240 native test/fixture lines, zero
production changes. Private Collision fixture owns a native winning transaction
and one JoinSet client so failure drops abort pending work. Force actual final
INSERT wait after first run staged, then commit winner: Conflict(index1,actual1),
only winner persists and new prefix key is absent. Dedicated no-DDL runtime role
then loses only keys SELECT while INSERT waits; metadata query must fail42501,
but exposed Store error must preserve ORIGINAL23505/expectedconstraint. Restore
permissions/close pool/drop owned test role before assertions. No synthetic codes,
new test runner, public adapter seam or runtime added.

PG5B3 classifier scope budget: <=95 native test/helper lines, <=8 production Rust
lines (still within cumulative core+210). A real mirror trigger violates another
schema.events/events_stream_id_version_key on an existing owned stream. Both raw
and atomic requests must retain Store(Sqlx23505) with native foreign schema, never
OCC. Test red before adding supported PgDatabaseError.schema() via fallible native
DatabaseError downcast. Exact upstream0.9 error.rs schema()/try_downcast_ref() read;
no custom message parsing, error replacement or new public variant. Record
failure, fix predicate, rerun positive native collision plus negative namespace.

PG3 real-failure proof (PG5B4) BEFORE changes: <=210 native test lines, no production
changes. Create actual SSI read/write cycle with two SERIALIZABLE transactions
and a test-only native trigger reading B then blocking after SUT inserts A;
controller reads A then writes/commits B, release SUT and require actual40001,
only B durable. Deadlock test: controller holds B, atomic SUT writes A and waitsB,
controller then waitsA; SUT shorter native deadlock_timeout, controller longer,
require actual40P01 and no SUT rows. Native locks and independent committed rows
must witness both. Do not use RAISE for these codes or call a controller-only
error proof of adapter handling. Existing fake deferred triggers remain mapping
coverage only. Fix failing schedules honestly; no skips or synthetic substitutes.

PG6 preparation (no implementation yet): separate aggregate snapshotsVersion and
projection checkpointsPgAllPos, share raw key catalog without copying IDs just
for lookup. Unconditional aggregate cache replacements differ from checkpoint
revisionCAS; reuse CheckpointWrite::next_revision as sole revision/schema/order
policy. Native key-row NO KEY UPDATE candidate permits event FK KEY SHARE; prove
with blocked checkpoint and real append, including absent-row creator races.
Schema3 must be feature-independent and versioned, preserve earlier positions,
validate metadata before stale return, condition body fetching on matching schema
and byte limit, allow empty state and all valid unsigned extrema. Save/read full
state+position pair in one SQL transaction/query. Error/cancel requires reload.
First add shared/native characterization against absent implementations.

PG6/PG9 input error finding: current validate_sizes maps input byte-count overflow
to CorruptRow. Arithmetic remains checked but failure domain is wrong: oversized
caller input is not persisted corruption. Avoid copying that classification into
state writes. Research a narrow common typed size-overflow error and shared byte
accounting before extending state storage; preserve original persisted corruption
detection. No saturating arithmetic, sentinel actual sizes or fake envelope.

PG9 minimization candidate: atomic prepared Vec currently retains (index, write
reference, key), duplicating index/reference already authoritative in writes.
Compare retaining only keys and zipping the original tail after all-heads phase;
measure request memory and keep ordering/race/mutation tests. Defer until final
minimization rather than change reviewed behavior without matching verification.

PG8 durability candidate: verify supported native Nix hook startup flags/settings
before claiming crash durability. Explicitly test fsync/full_page_writes/logged
relations and per-connection synchronous_commit; builder-only validation cannot
prove later pool connections' session settings. Reject incompatible write-session
settings before writes or use supported scoped settings with explicit operator
contract. Preserve independent replication/failover guarantees. No passing native
suite by itself establishes this profile, and no server setting may be silently
weakened to speed tests.

PG6 key-row NO KEY UPDATE candidate REJECTED before implementation: native42501
with existing SELECT/INSERT grants requires extra UPDATE on immutable key catalog.
Do not weaken permissions. Lock native checkpoint row itself; absent-creation
race needs valid full INSERT ON CONFLICT DO NOTHING plus observing winner's
revision, with sole next_revision authority for revision/schema/order policy.
First deliver aggregate cache snapshot feature/migration3 as PG6a (budget and
falsifiers in feature research), then conditional checkpointPG6b. Do not mark all
PG6 complete at the first sub-slice. PG5 PR381 checks/review remain pending.

PG8 security audit follow-up: inspect caller search_path with pg_catalog explicitly
placed last and custom equivalent operators/functions. Owned relation names alone
do not constrain unqualified builtin lookup; pg_get_constraintdef output may also
change qualification. Prove supported profile/SQL behavior before claiming namespace
independence. Consider scoped canonical validation and qualified native functions/
operators versus session changes; measure read/append overhead. No new tenant or
connection framework, and no caller-owned global setting changes without contract.

PG6b locked-owner design after PG6a: implement existing
CheckpointStore<Vec<u8>,PgAllPos> under projection, with migration4 separate from
aggregate cache. Initial bounded change <=260 production Rust added/30 removed,
<=100 SQL lines, <=500 native law lines and <=100 existing proxy lines; reserve
separate explicit consumer/privilege/recovery verification passes before retention.
Full unsigned revision/schema/position domains (zero PgAllPos valid), empty state,
metadata corruption before Stale and byte-bounded body selection are acceptance.
Read metadata under native checkpoint SELECT FOR UPDATE. Absent creation uses
next_revision(None) then a full valid INSERT ON CONFLICT DO NOTHING; lost creation
observes committed winner under row lock and returns the owner's exact conflict.
Existing record validates sole CheckpointWrite::next_revision authority then
replaces revision/schema/position/state in one transaction. No placeholders,
unconditional checkpoint API, extra key UPDATE grants or CommitUnknown retries.
Falsifiers: two real concurrent creators/updaters both win; loser reports wrong
actual revision; forbidden schema/position/rebuild/overflow changes any byte;
reader observes mixed pair; actual committed write after ACK loss becomes safe
retry; canceled write consumer continues without reload. Include real projection
stepper, native typed-codec errors, same-ID snapshot/checkpoint separation, actual
client kill and migration collision/repair. PG6b starts after focused PG6a source
is retained; this design is not runtime or whole-PG6 acceptance evidence.

PG9 source-filter minimization expanded: raw evidence JSON also enters the broad
fixture JSON filter, causing complete rebuilds on documentation-only amendments.
Exact tool output can be retained as a log without changing compiled sources or
skipping checks. Compare supported scoped filesets for actual fixture/SQL owners
against baseline derivation identity and required compiler/input tests; no manual
hash edits, skip gates or ad hoc filter framework. Current PG6a adds no filter rule.

PG7 position-context question remains OPEN: current public PgAllPos::new(txid,seq)
contains no database/history identity. Persisted profile checks may detect a
copied foreign database but cannot distinguish identical numeric cursor pairs
from different histories. Reproduce against two real databases before choosing
minimal adapter-owned context/serialization changes. Preserve supported in-place
upgrade data/positions, explicitly resolve legacy history without inventing lost
provenance, and measure representation cost. No new kernel API or chosen history
scheme is justified before that witness and compatibility decision.

PG9 audit evidence gap: actual candidate CI39130711194 crate-audit prints errors
checking yanked status because registry entries are unavailable. Green overall
check is not evidence that yanked status was verified. Inspect maintained audit/
registry behavior and exact inputs, keep vulnerability/license failures blocking,
and obtain actual final registry evidence without a custom tool or new ignore.

PG7a pre-edit (PG6b source7599588e/PR383 awaiting CI): implement only bounded,
finite StreamLister before consistent export. Existing RawEventStore supplies
EventExporter blanket; reuse export feature and BoxStream, SQLx keyset pages,
existing Limits/PageRow/corruption mapping. No new framework/runtime/dependency
or public type. Budget <=180 added/10 removed production Rust (including SQL
strings), one Cargo feature/module declaration, <=450 native test lines. Revert
listing owner/feature if finite visibility, one-per-event-key, page/body budget,
connection release or reopen laws fail; never weaken those tests. State-only keys
are absent. Ordinary listing freezes safe transaction cutoff, then keyset pages;
no whole-catalog fetch or connection retained while idle between pages. First
characterization is missing trait API (zero runtime), then real native tests and
an isolated unbounded-list control must reject actual whole-backlog loading.
PG6b parent delivery still must complete; no PG7 release/readiness claim.

PG7b pre-edit: consistent ExportSession uses existing ConsistentExporter and
ExportSession contracts, shared bounded list/event pages and one repeatable-read
read-only native view established BEFORE return. Budget<=450 added/80 removed
productionRust, at most1publicSession type and3typed dispatch/cleanup errorvariants,
zero newdeps/framework/runtime/tool; <=600 initial native lines. Compare supported
SQLx ownership/lifetime alternatives before retaining mechanism: ownedtransaction+
Mutex can be pinned by unpolled queryfuture; bounded activelydriven Tokio owner
can own borrowed Connection::begin_with plus public PoolConnection.close_on_drop.
Pool leases remain authoritative resource bound, no detach/leak. Expiry must
release idle/buffered/blocked views without callerpoll; close awaits cleanup,
opening cancellation before/after view creation leaves no view. Source original
missing trait counts0runtime, not actual cleanup/atomicity evidence. Native
fourcategories BEFORE subtle implementation; reject/refine owner if actual
backend_xmin, shared snapshot, cancellation, byte or resource laws fail. No custom
CancelRequest protocol (selectedSQLxpostgres has no located public cancel API).
No new histories/contexts until real foreign-restore witness demonstrates gap.

PG7b adversarial refinement before retention: initial five passing native laws do
not show cleanup of an ACTIVE lock-waiting read, nor caller-owned pool closure.
Native lock an events table after session key lookup, poll actual cursor once,
prove its pid wait_event_type='Lock', abandon next future but keep cursor/session
unpolled; unchanged1s lifetime must remove backend_xmin within5s BEFORE blocker
release. Always rollback independent blocker BEFORE failing assertion. Separate
idle60s session must let pool.close finish within3s; always drop session before
assertion. Any red reopens owner lifecycle design, no bound/test relaxation.
PG7b native red refinement: add supported LOCAL client_connection_check_interval
at1s (preserve shorter caller setting) before ready; native kernel socket check
must remove blocked view after driver closes it. This bounds disconnect detection
without shortening queries or changing lifetimes. Watch supported Pool::close_event
alongside drop signal in owner, so caller close is independent of60s session.
Original two reds unchanged; zero new leases/deps/knobs. Retain only if BOTH data/
resource assertions pass; partition behavior remains PG8, not inferred.
PG7b gate extension before edit:5lines in existing native Nix feature smokes to
require actual session close/drop/reopen selector with --no-tests=fail. No custom
runner/checksuite. Full203 includes actual sessions; missing export must fail
selector, default skip not a successful native session. Same product450/80,
600initial native budget. Update docs to disclose one pool lease/task per session,
per-cursor bounds, finite lifetime, LOCAL1s disconnect checks preserving shorter
caller interval, pool close/cleanup, retained native MVCC cost and network limits.
2026-10-18 user-requested PG9 release checks repair, isolated work/release-checks
from merged88a4575b, preserve PG7b PR385 f94333c7649a1a69e875728bc4a5553355f11fa7.
Actual releasePR378 head8836a37c run39147192934 all4AFLbuilds fail locked manifest
BEFORE fuzzing; neither retry nor removing --locked proves repair. Compare root
release version edits with selected isolated fuzz lock graphs, reproduce exact
cargo --locked resolution failure in release-head worktree, then supported Cargo
updates must preserve unrelated registry pins. Keep same4targets/durations/gates.
Pre-edit budget: <=80existing release-workflow lines,0productionRust,0newscript/
framework/tool/runtime,0manualversion changes inmain. Use supported release-plz
output and ordinary Cargo/GitHub APIs; separate checkout so publishing CANNOT use
unmerged releasePR head. App-authored verified commits, expectedHeadOid prevents
concurrent overwrite. Existing release workflow owns refresh; no competing bot.
Current transient repair may update releasePR locks with exact evidence; protected
PR378 MUSTremainunmerged until whole-adapter acceptance. Separate CodSpeed decode
-10.42% across3payloads needs actualsame-environment evidence, no threshold raise,
benchmark deletion or manualacknowledge. User explicitly requested fixing asgo.
2026-10-18 PG7c FIRST native logical-restore characterization, main12fd839c.
Actual PRD demands copied xid8 must not be assumed compatible with fresh cluster
history. BEFORE choosing history metadata/API, use real PG18.6 initdb/pg_dump/
psql to copy migrated events to an independent fresh counter. Actual adapter
append/read_stream/read_all are SUT; SQL transaction allocation only establishes
higher source counter. Expected coherent original stream/global visibility must
fail if copied xid is filtered as future. Budget <=220 native test/fixture lines,
0production/API/dependency/format changes for characterization. Independent Unix
socket directories, TCP disabled, same server version; fixture cleanup even on
assertion failure. Expose real original red first, then decide supported logical
reimport/rebuild guard and identity context from evidence. No direct raw-restore
support or context scheme promised by this initial probe. No custom project tool.
2026-10-18 PG9 release performance experiment, parent12fd839c. Three downloaded
actual CodSpeed base/head decoder traces (all payloads) show identical35ns
instruction cost and one extra simulated cache-miss pair, adding58.333ns. Version
PR changes no decoder/benchmark source, but compiler may place/optimize its body
differently. Experiment ONLY supported #[inline] hint on existing public
wire::decode_frame, not inline(always), not benchmark alteration. This makes body
available across crate boundaries; LLVM remains free to choose. Before editing,
measure unchanged actual existing three decoder workloads on pinned native Rust,
then same command after single hint, inspect binary size and existing decoder/
wire laws. Budget <=2productionRust lines,0API/dependency/runtime/test/benchmark/
gate/threshold changes. Falsifier: runtime regression outside measured noise,
code-size cost outweighs actual improvement, any changed error/decoder behavior,
or unchanged release performance failure. Native ARM alone cannot prove x86
CodSpeed result; retain only after all workload/CI/release-head evidence. Rollback
single attribute; original failure remains in records. Do not accept a green
benchmark by dropping cases, changing metric, cold-cache policy or threshold.

2026-10-18 PG7c history experiment BEFORE product edits. Real native data-only
restore red and PG17/18 unprivileged native identity witnesses justify context.
Select one persisted16byte history generation plus native cluster/database
binding. Native gen_random_uuid/uuid_send and SQLx0.9 fixed-array BYTEA decode
reuse supported server/library facilities,0new dependencies. Native IDs alone
cannot express a new logical history recreated within the same database.
PgAllPos becomes history+txid+seq; one public History value with byte constructor/
accessor and store/position history getters supports actual serialized consumers.
Constructor now requires history (experimental0.x breaking minor per STABILITY;
conventional breaking commit/release-plz handles version, NOmanualmainbump).
Do not add an unscoped/zero sentinel fallback or silently attach foreign cursors.
Migration5 ONLY adopts trusted current legacy rows under explicit migrate; old
external2scalar cursors require explicit provenance verification before adoption.
Checkpoint rows preserve history with revision/schema/state/position in SAME CAS
transaction, never retag copied state. Physical recovery keeps coherent metadata;
logical completed-CBOR reimport creates fresh history and projection rebuild.
Bounded startup max-xid comparison uses captured snapshot XMAX (NOT xmin), native
index ORDER BY txid DESC LIMIT1; raw copied future xids reject before use. Native
binding copied to another cluster/database rejects too. Caller pool identifies
one database; reconnect/failover profile remains PG8, not inferred here.
Pre-edit budget <=300added/100removed productionRust, <=110SQLlines including
newmigration5/schemaaudits, one new public type/3error variants/no kernel changes,
<=500new native testlines including189existing characterization, <=90old callsite
or fixture edits for explicit scope/metadata, no assertion/limit/lint weakening.
Falsifiers: old committed values/txid/seq/state/revisions change onupgrade; same-
history ordering/resume changes; foreign cursors accepted; raw copied profiles/
futurexids open; physicalcoherentrestore/reopen breaks; profile/grants allow
runtime rewriting identity; bounding/runtime/allocations regress without measured
tradeoff. Measure position size16=>expected32 and native bounds/runtime; explicit
16byte persisted history cost added to checkpoint read/write byte accounting.
Update every exact boundary fixture to SAME configured ceiling with correspondingly
shorter body/known overhead, NEVERraise limits to green. Before retain, demonstrate
fresh native controls disabling physical/future/position guards fail actual laws.
Rollback new5/history position changes together; never rewrite migrations1–4.
PG7c byte-edge fixture clarification BEFORE edits: checkpoint fixed accounting
now48bytes, so old37byte ceiling cannot fit EVEN an empty row (one-byte key
requires49). Preserve37 as an explicit unchanged too-small-floor negative and
for stale/oversized transfer tests. Exercise exact new positive boundary at53
with four-byte body and54overflow, not by claiming old37 still stores that row.
Production limits are unchanged. This new pair of laws is stronger than simply
raising the old fixture limit. Every original body/CAS/state oracle stays.
PG7c migration5 populated-v4 follow-up BEFORE new test edits: first209 green
suite has no checkpoint row before5 (earlier fixture startsv3 then installs4+5).
It therefore does NOT prove promised backfill. Allocate an additional<=80native
lines, cumulative new-test budget<=580 (initial500 retained),0extra product/SQL/
API budget, for ONE populated-v4 collision/rollback/repair/unsigned-state proof.
This strengthens missing acceptance instead of inferring it from green tests.
Original migration1–4 checksum/rows remain exact or experiment fails.
PG7c final native controls and Linux211 evidence support retaining the history
candidate pending the unchanged full gate and protected delivery; no completion
claim. Whole PG7 operator drill must independently assert old-primary shutdown
(the current physical fixture invokes pg_ctl stop through Drop but ignores its
exit status), actual Projection restart, bounded completed chunks and logical
rebuild. These remain explicit falsifiers; current physical-copy coherence is
not failover fencing. Current product source freezes through final gate/CI.
