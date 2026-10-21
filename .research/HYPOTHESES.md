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
# PG8 global read search path (2026-10-18)

Threatened invariant: caller pool search paths must not change the safe global
transaction cutoff or exclusive resume comparisons. Workload: two real writers
with an older uncommitted event and a newer committed event; a separate reader
pool puts a test-owned schema before pg_catalog. Independently shadow the two
snapshot functions and xid8 comparison operators. Require empty reads while the
older writer is held, then exact ordered events and exclusive resume, including
u64::MAX sequence resume. Gather outcomes and remove injected objects before
asserting, so a failing witness cannot poison subsequent cases.

Pre-edit budget: <=200 new native-test lines, <=20 production Rust/SQL-string
lines, no public API, dependencies, migrations, kernel, runtime or tooling edits.
Falsifier: all four original-source native witnesses pass unchanged. If they
fail, qualify only the proven builtin functions/operators, then re-run the same
oracles and relevant native/conformance checks. Rollback boundary: this slice's
test and store-query edits only. Research/PR delivery records are separate.
# PG7 completed bounded backup composition — 2026-10-18

Before logical-restore consumer law: refine test-only ceiling550->650 to reuse
the existing actual Counter projector/repository fixture through backup, fresh
history rebuild, pool close/reopen, saved-checkpoint reload and subsequent append.
This addresses explicit PG7 restart acceptance. Other budgets stay unchanged.
Physical operator-stop and capacity remain separate.

Before adding native failure/cancellation cases: extend the test-only ceiling
from500 to550 lines for two actual blocked-query faults and their native wait
observation. Production0/dependencies2/lock-edge5 budgets remain unchanged.
First four laws are356lines; the added cases must use actual session cursors,
not synthetic failing streams, and preserve original native failure causes.

Threatened invariants: one finite export view, completed bounded format2chunks,
exact per-stream fields and continuation versions across chunks, logical restore
assigning destination positions and rejecting source positions. Compose existing
store/CBOR/session/import facilities, using lazy listing, two-event batches and
one fixed16KiB sink reused per completed chunk. Restore reads at most that fixed
size per file; reject oversized/truncated input before importing it. No whole
catalog or whole backup materialization in the backup/restore paths.

Pre-edit budget: <=500 native-test/helper lines, two existing-workspace dev-dep
entries/feature edits, <=5 lockfile dependency-edge lines, no production/runtime/
framework/tooling/newformat changes expected. Existing minicbor2.2.3 supports
&mut[u8] sinks, ChunkWriter finish recovers unused capacity, existing Fjall
adapter supplies the interoperability peer. Production dependency cost stays0;
both dev dependencies already exist in the locked workspace. Falsifiers: actual
roundtrip/interchange loses any field, bounded sink overflows accepted workload,
normal decode accepts incomplete data, or a failed chunk is falsely completed.
Any production defect needs a separate recorded budget before editing its owner.
This first pass is not the complete PG7 operator/restart acceptance proof.
# PG7 physical operator recovery — 2026-10-18

Before sharing the existing logical consumer with physical recovery: allow removal
of its35-line private duplicate plus up to15 import/module/call edits in the
backup owner, in addition to the physical slice's220newRust/15restore edits.
Both actual consumers must retain exact pair4/restart/advance5 assertions. One
private test helper now has two concrete owners; no new public/product interface,
dependency or alternative projection implementation. Rollback is these test-only
edits if either owning recovery law or feature compilation changes behavior.

Before native backup-integrity case: refine new test/helper Rust ceiling180->220
for supported pg_verifybackup (manifest/checksums/WAL) and an actual same-length
file corruption/missing-file witness before boot. Existing15-line/document100/
production0/dependency0 ceilings unchanged. This replaces no parser and introduces
no custom backup format. Initial six restore laws already passed180.715s.

Threatened acceptance: a best-effort Drop is not proof that the old primary
stopped; raw checkpoint bytes are not an actual restarted projection consumer.
Use existing independent18.6clusters, pg_basebackup, pg_ctl and actual Counter
repository/projector. Verify successful waited stop and native status3 before
booting the copy. Preserve migration checksums and saved projection pair, reload
through the real consumer, then append and advance again. Record backup and
recovery elapsed times as this fixture's measurements, not deployment targets.
Falsifiers: source still running, checksum/history/state mismatch, restart replay
duplicates, or later append cannot advance. Explicit native control must reject
a skipped old-primary stop. No cross-node failover/fencing/power-loss claim.
Pre-edit budget<=180new test/helper Rust lines,<=15existing test lines changed,
<=100operator documentation lines;0production/API/dependency/runtime/tool changes.
Keep current backup commit frozen. Add small shared consumer fixture for the
logical/physical recovery owners; use it in logical tests after parent delivery.

# PG7 completed backup routes and failure boundaries — 2026-10-18

Previous goal turn PROGRESS: PR388 protected merge, signed backup PR389, shared
physical/logical consumer plus224 actual native laws; live Linux31050 and full
signed hook95312 verified, not restarted. Continue independent acceptance while
those gates execute; do not mutate their worktree/index.

Threatened invariants: completed CBOR must retain caller binary routes and exact
fields; a late duplicate rejects before writes under both policies; a final
storage-head conflict leaves WholeChunk untouched and PerStream retains exact
reported earlier progress. Consistent CBOR must retain one session view even
when an actual overlapping atomic writer commits between exported streams.
Normal import rejects damaged/torn artifacts; deliberate salvage retains producer,
completion uncertainty and exact recovered prefix. The caller's independent file
inventory and fixed chunk cap must reject missing/oversized files before writes.

Pre-edit budget: <=450 new native-test/helper lines and <=45 existing helper/call
edits in backup_tests.rs,0production/API/dependency/runtime/tool changes. Reuse
fixed16KiB sinks, real Postgres/Fjall/store import, existing CBOR APIs and native
Barriers/atomic fixture. Read one chunk at a time; no whole-backup collection.
Falsifiers: any mismapped/lost field, early write on ambiguous route, wrong final
conflict/report, mixed cross-stream source view, silent normal salvage or absent
file accepted using only completion evidence from the remaining files. Each test
must distinguish intended native/typed failures from setup errors. No new backup
format, manifest framework or production file orchestrator. File inventory is
explicit caller evidence, not something completion records can infer. Rollback
boundary is this test/doc slice if the actual owners cannot satisfy the contract;
record a separate production budget and original-source witness before a fix.

Before adding independent inventory and explicit salvage laws: refine new-test
ceiling450->650 for completed-chunk source plus native malformed/CRC/salvage,
missing-final-file and cap-edge checks. Current four laws/helper293added/1removed.
Existing helper-edit ceiling45 remains: make read_chunk/restore fallible for actual
file errors; restore first checks the caller's complete independent inventory,
one bounded file at a time, before any import. It re-reads verified immutable
files during import; no concurrent filesystem-mutation or whole-backup atomicity
claim. This is test composition of existing primitives, not a production manifest,
backup runtime or new tooling. Completion cannot infer missing files; expected
count is independent caller evidence. Keep exact IO kinds and contents assertions.

Before clarifying the demonstrated report ambiguity: allow <=15 core doc lines
only, no shared executable change. Existing canonical import laws and adapter
atomic tests pin reachedNone when this request commits nothing, even for a
nonempty target; do not change those semantics. Clarify request progress vs
current target head, retain independent exact destination contents assertions.

# PG9 decoder call boundary — before edit, 2026-10-19

ReleasePR378 and TLS PR392 both fail the same original decoder workloads:
501.7->560ns,10.42%, each of64/1024/65536payloads. Existing downloaded profiles
show identical35ns instruction cost and one extra L1/last-level cache miss in
public decode_frame. Placement/alignment is a hypothesis, NOT proven cause.
Existing outer #[inline] candidate was rejected25–36%slower/+512binarybytes;
DO NOT repeat it or alter workloads/threshold/cache/metric to pass.

Inspect actual production owner: decode_frame reads FrameHeader, chooses its
body header size, then passes both the header and redundant derived size to a
single-use private decode_frame_body. Hypothesis: move format-dependent size
selection into the body owner that uses it, removing the redundant argument and
public wrapper work while retaining one common checked body decoder. No new
abstraction. Candidate budget<=18existing Rustlines changed,0new API/deps/methods/
benchmark/workload/inline hints/gate changes. All error values/priority, V1/V2,
metadata None/Some(empty), padding/bounds and arithmetic safety unchanged. If
function length exceeds80, reject/refine without a lint allowance.

Before source change measure CURRENT unchanged parent49104117 through actual
existing Fjall decoder benchmark in pinned Nix/Rust1.99, release, three original
payloads,30samples,1swarmup/3smeasurement, repeated twice for current noise. Use
supported Criterion named baseline if available; preserve exact current baseline
binary. Do not measure during own native regression jobs. Existing prior ARM
baseline is reference only; no attribution of CLI comparisons against old saved
failed-inline samples. Then one causal code move, same workload repeated, exact
wire/product tests/fuzz and binary size. Reject native regression or semantics
change. Native ARM cannot waive x86 simulation: CI+exact release head must pass.
If head-specific placements remain sensitive, investigate further; never manual
ack/reset/threshold or easier benchmark. Preserve existing performance worktree
and records unchanged; new isolated /tmp/mnesis-decoder based protectedmain.

# PG9 overflow error paths — next independent experiment, 2026-10-19

Body-boundary move REJECTED/reverted; original saved-binary measured A/B/A
restored lower native runtime. Next credible cost: decode_frame_body repeats the
same OffsetOverflow construction11times, including eager ok_or expressions.
Hypothesis: one private cold error constructor and lazy ok_or_else preserve exact
error fields/priority while keeping exceptional code out of the hot path. Standard
Rust #[cold] is a supported hint, not proof; no annotation on normally hot code.
Budget<=60Rustline churn, one<=8line private function,0new public API/deps,
0benchmark/input/threshold/gate/inline changes. Start from original parent code,
not rejected candidate. Native baselines and saved original already recorded;
use same actual three workloads/30samples/warmup/measurement and named actual
parent-return baseline, inspect binary size, then actual wire/property/fuzz,
strict and full checks IF performance permits. Reject real native regression,
any changed error/format behavior, unnecessary code cost or unchanged final
release failure. Native ARM alone never waives x86 check. Rollback one ownerfile.

# PG9 fixed header fields — independent experiment, 2026-10-19

Both body-boundary and cold-error candidates rejected/reverted. Actual parent
LLVM decoder includes six panic_bounds_check call sites after an explicit header
length check. Current reader chooses dynamic per-field offsets and does byte-by-
byte indexed loads; compiler has not eliminated their bounds machinery. Hypothesis:
use maintained core slice::first_chunk and an exact array pattern for the common
fixed schema/event-length/metadata fields, choosing only the V1/V2 starting point.
An array reference proves field width before loads, removes indexed arithmetic
and unreachable panic paths, with no unsafe/custom codec/dependency.

Before edits budget<=100Rustline churn,0new public API/format/deps/framework,
0new generic helper. Preserve initial minimum11 priority, unsupported-version
priority, exact V1minimum19, endian fields, absent/empty metadata and all decoded
ranges/errors. Remove two obsolete V1 internal offset constants; keep V2 offsets
only for existing test fixtures if no production consumer remains (verify all
references first). Type/pattern determines common fixed field width; no competing
layout registry. All size/offset arithmetic safety preserved. Existing exact
wire/property/fuzz/consumer laws must pass. Same original actual benchmarks and
named baselines; inspect native code size/bounds removal, reject slowdown beyond
noise, CI x86/release proof still required. No hints, workload/metric/cache/gate
changes. Rollback ONLY ownerfile. First_chunk is stable core facility on pinned
Rust1.99; verify exact local primary docs/source, not an external new crate.

# PG4/PG8 caller query names — original witness before edits

Prior global-order repair qualified only its snapshot functions/operators.
Current store read_key/read_head still use unqualified bytea equality/MAX;
caller search_path may explicitly place application schemas before pg_catalog.
Threat: a normal application operator can redirect raw stream identity/head and
state lookups, despite schema-qualified tables. Source finding is not proof.
First budget<=160 native test lines,0production/API/SQLschema/dependency/tooling
changes. Real existing PostgreSQL, healthy migrated store, two different streams
with exact bodies. Max2pool and native LISTEN readiness ensure one data slot;
change ONLY its search_path after successful native controls. Native custom
bytea equality returns true while explicit pg_catalog equality is false; actual
read_stream(second) must still return ONLY second's exact event. Drop injected
schema BEFORE assertions to prevent poisoned subsequent cases. Use default SQLx
statement caching; no synthetic driver. Original source must fail in routing,
not fixture setup/query syntax/arbitrary error, before any SQL changes.

Follow-up only after actual red: pin all load-bearing function/operator/type
names at owning SQL source, preserving caller settings/pool ownership/order/
byte bounds/atomicity/outcomes. Do not impose session GUC mutation, wrap every
cursor in long transactions, add stored-procedure framework or change runtime.
Separate budget/native witnesses for head forgery, byte accounting and schema
validation/deparser concerns. Reopen feature-complete acceptance only if actual
contract defect is reproduced; original whole goal remains active regardless.

Original native byte-equality witness is now RED at the exact second-body oracle:
read_stream(second) returned first-body, one row, after healthy control and native
operator selection controls. Before further edits expand test ceiling160->320
for distinct head-forgery and state-write routing laws. Production remains ZERO
until their original-source execution. A forged max(numeric) must not make an
append expected999 accept event1000 over committed head1. A snapshot for second
must not replace first's state even if application bytea equality is visible.
Cleanup injected schemas before asserting captured results. Inspect actual native
rows after cleanup; no synthetic read oracle. First repair budget<=35 changed
existing Rust lines in read_key/read_head and state lookup SQL only, no API,
schema, dependencies or caller session changes. Byte accounting, integer joins
and catalog validation remain separate unresolved hypotheses with new witnesses
before expanding this budget.

Before checkpoint-read witness expand new-test ceiling320->360 for one independent
projection pair lookup (owner separate from snapshot hydrate). Use actual read_all
positions from the two acknowledged seed events; preserve exact revision/state/
position oracle. No production fix until native original5laws run. Existing SQL
repair ceiling35unchanged. No custom test framework/helper configuration needed.

Before full gate add one missing observable to the same5 native laws: after each
SUT query, caller search_path must still equal the configured path. Capture SHOW
on the single data connection, drop the injected schema, then assert captured
path and original exact product result. Small private cleanup helper removes
repeated schema-drop SQL; same360test/35production budget. This rejects an
incorrect session-reset workaround that could otherwise pass payload assertions.
No new production change or test-only API; repeat native/full Linux on finaltest.

# PG9 check-output cost — measured candidate, not implemented

The concurrent query/TLS gates exhausted disk; original StorageFull and compiler
failures remain recorded. Serial retry passed unchanged. Query nextest output
archives4.28GiB of target into1.01GiB although its declared consumers require the
check verdict/coverage, not its target archive. Locked Crane47b6b27 source
cargoClippy.nix/cargoNextest.nix/mkCargoDerivation.nix supports the existing
`doInstallCargoArtifacts = false` option; nextest partitions=1 here. Repository
search finds all cargoArtifacts inputs owned by buildDepsOnly, not these checks.
Candidate after focused query/TLS delivery: two check-local option lines, no
product, dependencies, commands, filters, properties, coverage or native archive
changes. Preserve buildDepsOnly and postgresTests outputs. Falsifier: any actual
consumer needs either removed archive, test selection/command changes, coverage
is absent, or unchanged full local/Linux gates fail. Measure actual output sizes,
checks/coverage and repeat costs before retaining. This is standard Crane reuse,
not a new tool or relaxed gate. No implementation or saved-space claim yet.

# PR394 mutation evidence rejected — cleanup before failure (pre-edit)

Actual downloaded CI artifact12161164145, digest68224bde5803893cd4183e9364e963ed98c3ee7b029355aef063fc1e51a7f0af,
reports16mutants:14caught/2unviable. Independent log inspection finds EIGHT head
variants actually stop at42P06 ordering-schema-already-exists after the first
head(0,None) variant panics inside the overlapping append. These eight are NOT
credible catches. Baseline235PASS49.299s; read_key four and first two read_head
variants have real runtime failures, two hydrate variants fail Default compilation.
Original reports/logs remain unchanged. Existing shadowed_ordering helper unwraps
the spawned append before native cleanup. Candidate: capture append and join
outcomes, drop native probe before unwrapping; <=8changed TEST lines, zero product,
framework or policy changes. Preserve overlap and every order/resume assertion.
Falsifier: repeated failing head controls still leave ordering schema, produce
42P06 instead of their own SUT failure, or healthy native/full suite fails. Prove
original repeated-control failure, then exact same controls with fixed cleanup;
rerun actual mutation CI and inspect every first failure. Do not merge on current
reported14caught; no waiver or fixture-failure credit.

# PG8 verified TLS and runtime permissions — pre-edit2026-10-18

Threatened invariant: SQLx caller pools must support actual certificate/hostname
verification, authentication and least-privilege runtime access, with no plaintext
fallback in the production example. Current exact0.9 manifests enable no TLS
backend; source inspection is not a runtime witness. First add a real private
native SSL server/CA and require VerifyFull to succeed. Establish that it fails
for missing TLS support before selecting supported SQLx TLS features. Retain
positive controls for all negative CA/hostname/auth cases and exact native causes.

Initial budget:0productionRust/API/dependency changes before witness; <=650new
native test/helper lines, <=100existing native-cluster lines moved into one
private support owner plus <=30restore caller edits. Reuse common initdb/pg_ctl/
directory/cleanup at its second consumer; keep restore-only backup methods with
restore tests. No public test seam, custom server framework, runner or tooling.
<=3Nix package-list edits add existing nixpkgs OpenSSL CLI where native fixtures
execute; pinned Nix owns all tools. Do not use a host OpenSSL or install ad hoc.

After actual witness, a separate record must compare SQLx-supported Rustls/ring,
native-TLS and AWS-LC selections for fit/license/maintenance/advisory/dependency/
build cost before changing the adapter's one Cargo TLS feature. Expected no
productionRust/API change, <=20new transitive packages/250lock lines; explain
actuals/revise before expansion, never weaken gates. Required tests cover saved
lifecycle/pool ownership, wrong CA/hostname/auth and redaction, no TLS downgrade,
restricted role append/read plus rejected DDL/event mutation, and actual
Barrier/spawn competing appends over verified TLS. Final complete native/Linux/
feature gates, release consumer and durability/operations evidence remain due.
Rollback is this slice only if supported SQLx facilities cannot meet the contract;
never retry uncertain writes automatically or add a custom TLS stack.

After actual native MissingTLS witness, select SQLx0.9's supported
 tls-rustls-ring-webpki feature for the adapter (one existing Cargo feature edit,
0new public API/productionRust/runtime). SQLx owns handshake/hostname/cert parsing.
Ring's supported0.17 line avoids AWS-LC's extra build/provider dependencies;
WebPKI roots are reproducible across supported Darwin/Linux rather than loading
ambient OS roots. SQLx appends explicit ssl_root_cert to its built-in roots;
document this scope rather than promise CA-file-only trust. Native-TLS adds
platform-dependent SecureTransport/OpenSSL/SChannel; no tested need for alternate
protocol support. AWS-LC/PQ/FIPS profiles remain separate claims, not defaults.

Exact dependency floor for this delivery: current RustSec Oct14 advisories0346–0349
all patched Rustls>=0.23.46 (not stale cached releasespage0.23.45); live official
GitHub releases/latest confirms0.23.46 published2026-10-14. Ring AES advisory0009
patched>=0.17.12, old unmaintained<0.17 not selected. Audit and deny actual resolved
lock; do not add ignores or accept an older patch. SQLx0.9 MSRV1.94 fits pinned1.99;
Rustls/ring supported ecosystem, MIT/Apache/ISC-compatible, WebPKI-rootMPL2 allowed
by existing deny policy. Verify exact downloaded licenses and actual lock costs.
Expected<=20newpackages/250lock lines, no unrelated updates. Measure first build
cost and owning runtime tests, then full native/Linux/feature and consumer proof.

Resolved-library evidence revises root selection BEFORE next edit: WebPKI roots
current1.0.9 require unapproved CDLA-Permissive2, so reject that candidate without
changing deny policy or downgrading trust data. Select supported Rustls/ring
native-roots instead; configured custom CA and VerifyFull stay unchanged. Actual
native certificate+name semantics, negative controls and both OSes still required.
Root sourcing is deployment-owned OS/native trust plus explicitly added CA, not
bundled Mozilla set or exclusive custom CA. Allow<=24newpackages/350locklines for
maintained platform certificate readers (SecurityFramework/Schannel/openssl-probe)
while preserving0productionRust/API,1existingSQLx feature edit,2Nix CLI entries.
Native-TLS/AWS-LC not chosen: Rustls provider and TLS1.2/1.3 remain the same, only
supported root loader changes. Re-evaluate actual license/maintenance/advisory/
build/runtime cost. No weakened verification, gate policy or old-root pin.

Before correcting TLS race fixture: allow<=5 shared AppendError doc lines only,
0executable changes. Diagnostic-only run proves expectedNone/actualNone native
constraint collision. Existing PR377 forced-collision laws explicitly preserve
unknown actual until reload; preflight conflicts may instead observeSome1.
Require exact stream/caller expected and only those two permitted observations,
plus independent one-winner/exact committed event oracle. Do not force scheduling,
change the established error mapping, accept Store failures, or infer empty from
unknown. Document this existing optional observation once at AppendError owner.

Before final native CI wiring/generation: allow<=10existing checkPhase lines for
one --no-tests=fail TLS smoke using the existing integration archive, alongside
existing capability smokes; absence of the security binary/law must fail. Retain
normal complete native run and all current gates. Regenerate existing managed
workspace-hack through cargo-hakari after TLS dependency change; expected<=25
managed manifest lines plus corresponding local lock edges. No manual generated
manifest policy edits, new runner, framework or replacement tooling. Revisit
actual cost if generation exceeds this ceiling, not by skipping verification.

Before public example/documentation: budget<=70 new adapter example Rust lines,
<=120 public README lines plus PRD/backlog/evidence updates. Use existing SQLx
PgConnectOptions VerifyFull and an explicit CA file with caller-owned pool, no
new dependency/interface/URL normalizer. Separate migration privileges from
runtime open. Do not log DSNs/options/passwords. Explain native OS roots plus
additional CA, exact returned error/source semantics, restricted event grants
and state-only UPDATE rights. Example must compile in existing Cargo/Nix gates;
actual native security tests own failure injection, not the example.

# PG8 durability profile — original witness before production change

Writable-primary acknowledgement must include local WAL flush in the supported
profile. PostgreSQL18 runtime-config-wal documents non-off synchronous_commit
modes wait for local flush, while off can acknowledge transactions subsequently
lost on crash. fsync/full_page_writes and honest storage remain separate server
requirements; cross-node failover not established. Startup alone cannot prove
settings on a different/future pooled connection. Preserve stronger replication
settings; never overwrite remote_apply with on or label a commit timeout safe.

First witness budget<=70 new native test lines, zero production/API/lock/tool
changes. With real PostgreSQL and native SHOW controls, prove healthy on open
succeeds and asynchronous caller-pool open rejects. Current source appears to
validate only schema. Witness must fail if actual unsupported off opens normally;
a generic baseline error is not final typed rejection evidence. Record actual
result before choosing a minimal private transaction-owner/profile policy.
Subsequent witnesses must cover a changed/replaced connection AFTER successful
open, all four write owners, restart, and exact rejection/no writes. A new named
configuration error would require separate pre-edit budget and exact assertions.
No crash/data-loss claim from SHOW alone, no forced server setting or storage claim.

After observed startup red, extend pre-fix native budget<=310 total test lines
for actual REPLACED pooled connection after successful startup, covering append,
atomic append, snapshot and checkpoint separately. Existing SQLx set_connect_options
updates FUTURE connections only; observe native old/new backend IDs and exact
on/off settings. Reserve listener deterministically via native pg_stat_activity,
capacity2 leaves one data connection, close it then require different replacement
PID. Prove normal event acknowledgement first, then unsupported writes must
reject and preserve exact independent state after pool shutdown settles rollback.
Use one small private fixture, no proxy/runtime/framework. Production still0
until these original-source laws execute. A pre-write startup check alone cannot
satisfy these cases. Do not alter stronger replication settings or make physical
power-loss claims. Final errors must be specific, not arbitrary is_err greens.

After all5actual original reds, minimal production experiment budget<=80 new
private Rust lines, <=25 existing caller/enum/module lines changed,0dependency/
migration/config types/new public methods. One new PostgresError durability
variant names the rejected setting. Existing native current_setting is the
supported authority: reject off for fsync/full_page_writes/synchronous_commit,
preserve all non-off commit modes including remote_apply unchanged. Check startup
on its existing scoped connection and immediately BEFORE each actual COMMIT on
the same write transaction, after database-side effects/triggers. One private
owner validates and commits; replace four caller commit blocks, preserving
PostgresError::commit unknown-outcome mapping and original SQLx sources. A check
failure before any COMMIT rolls back staged work, never sends a fake success.
Do not build an independent transaction/retry framework or override caller GUCs.

Falsifiers: any baseline five law stays green/acknowledged; exact unsupported
setting not retained; rejected staged event/key/cache/checkpoint becomes visible
after pool cleanup; stronger replication setting rewritten; server-rejected or
lost COMMIT response changes existing PG3 classification. Add actual native
trigger changing setting during write and fresh runtime check-omission controls.
Profile check is configuration evidence, not proof of honest disk/power loss;
server reload/operator configuration remains trusted deployment ownership.
Measure extra roundtrip/alloc costs with representative writes before claiming
performance parity. Every existing native fault law and full feature/Linux/gate
must pass; final startup/commit checks cannot replace restart/recovery/soak work.

Before next durability tests: revise native budget<=450 total lines (current258)
to cover preserved on/local/remote_write/remote_apply, AFTER-INSERT setting change,
and independent unsafe-server profiles after shared cluster owner delivery. The
310estimate covered replacement only; no acceptance dropped. Rollback same
slice,0new dev/runtime dependencies. Final checked source budgets and deliberate
check-omission controls remain required. Keep connection settings at owning pool;
use bound native set_config for controlled experiments, not a new policy object.

Source-budget review after first retained candidate: private owner32/80lines;
existing source18added/11removed=29churn versus estimated25. Four extra import/
replacement lines were omitted. Revise existing-source ceiling to35churn before
further changes; scope remains one private validator/commit owner and one error
variant with no API method/type/dependency/framework addition. No duplicate GUC
policy or weakened commit classification justifies a smaller-looking alternative.

Before unsafe-server and primary-crash proof: revise test ceiling<=600lines plus
reuse of the existing66line private cluster owner from TLS slice (exact shared
file, no new machinery). Current354lines cover pool settings/trigger only; two
independent private server settings and actual immediate-stop/restart recovery
need explicit native process/status and persisted-state assertions. Keep helpers
private, max80lines/function, no mode flags/tool/framework. Source dependency on
TLS test owner is explicit; protected rebase onto TLS before final delivery.
Server settings: healthy migrate first, stop checked, restart with fsyncoff or
full_page_writesoff, native exact control then typed runtime/migration rejection
with installed metadata preserved. Separate healthy-profile immediate server
stop/restart recovers acknowledged events/history and, when enabled, snapshot/
checkpoint pairs, then permits subsequent append. This establishes PostgreSQL
process-crash recovery only, not OS/hardware power-loss, replicas or failover.
The production32line owner and existing29line churn remain within revised35.

Fresh guard audit: native current_setting functions are qualified, but their
text equality operators are not. Prior PG388 proved caller search_path can put
application schemas before pg_catalog. Hypothesis: a matching application
text=text operator can suppress guard rejection AFTER successful startup. Before
any fix add<=40native lines; revised test ceiling650 from600 to cover this new
credible concern (current591). Use actual existing off replacement then inject
only data connection search_path/native text operator, not schema-validation
fixture failure. Preserve exact expected Durability setting and settled row map.
Production remains unchanged until actual red. Minimal potential fix only three
native operator qualifications, +3/-3existinglinechurn (29->35 current ceiling),
0extra interfaces/deps/mechanisms. Reject control if it fails only setup/query
syntax or returns arbitrary schema error. No fake supported-search-path claim.

## PG8 deferred trigger durability — before test edit

Current private commit helper validates synchronous_commit AFTER writes but
BEFORE COMMIT; a DEFERRABLE INITIALLY DEFERRED constraint trigger may change it
only during COMMIT. Source hypothesis only. Extend existing immediate-trigger
native law with a second deferred schedule on a fresh Fixture, same schema
cleanup BEFORE assertions, independent settled counts BEFORE typed rejection.
No weaker oracle. New test stays<=650lines; no production change before actual
red. Native one-test run only after current query235suite terminal. If reproduced,
consider native SET CONSTRAINTS ALL IMMEDIATE inside owned transaction before
last durability validation, leaving original commit error mapping untouched.
Private module80line budget retained, existing caller churn35 unchanged; no
runtime/API/schema migration/tool/dependency changes. Need exact primary docs,
original red, all write-owner outcomes and strict checks before retaining.

# Durability integration: deferred finalization must preserve outcome policy

Current full native run fails old exact40003 oracle after deferred checks move
from COMMIT to SET CONSTRAINTS. New drain currently maps every failure to Sqlx,
which loses the existing conservative finalization boundary. Candidate: ONE
private mapper line reuses PostgresError::commit; no original test change, no new
public method or cause erasure. Preserve explicit23514/40001/40P01 rollback and
40003unknown. Clarify public error docs to include conservative deferred-finalization
uncertainty before explicit COMMIT; do not claim this40003 fixture proves committed
rows. Budget remains <=35existingchurn and80private lines (current29/37; docs <=4extra).
Falsifier: unchanged four-code native oracle still fails, actual after-COMMIT lost
response/timeout becomes safe, or any healthy/rejection/durability native law fails.
Full250 must rerun after targeted existing outcome suite; no exclusion or timeout
change. Unknown is conservative; it never proves a commit occurred or retry safety.

Durability documentation budget before editing: at most45 README lines and
3Unreleased changelog lines plus status/progress updates, no additional Rust/API/
SQL or tools. Describe exact startup/write guards, preserved stronger modes,
conservative finalization uncertainty, measured process-restart boundary and
unchanged caller settings. Keep hardware/failover/capacity and final release
acceptance explicitly pending. Existing release-plz still owns version publication;
no guessed released version or stale quickstart-version repair in this slice.

PG3/PG8 phase correction, pre-edit budget (2026-10-19): deferred constraints
now execute before the adapter sends COMMIT. A failure at that point proves
this owned transaction was never committed, independently of the SQLSTATE.
Reject the conservative drain→CommitUnknown candidate despite250green tests:
it discarded known phase to satisfy an old fault fixture. Retain its evidence.
Falsifier: same real deferred40003 must be Sqlx with independently zero rows;
current conservative candidate must fail this precise phase law. Preserve
literal-COMMIT mapping and all lost-ack/timeout laws. Reuse the EXISTING private
frontend relay for a native SQLSTATE rejection observed only when the client
actually sends COMMIT; this is an injected boundary error, not a real SSI or
server-commit campaign. Verify the client boundary observation explicitly.
Budget: production existing Rust stays<=35line churn (restore original unknown
outcome docs, pre-COMMIT error Sqlx); existing outcome test/relay<=150line churn,
no new fixture framework/runtime/API, no assertion/exclusion/workload relaxation.
Rollback only this phase experiment, preserving every red and source snapshot.

PG1/PG4/PG6/PG8 byte-accounting experiment, pre-edit (2026-10-19): bounded
readers must enforce the native byte limit even when the caller explicitly puts
pg_catalog after an application schema. Source has bare octet_length(bytea) in
page/listing/snapshot/checkpoint readers; table qualification alone does not fix
function resolution. SOURCE FINDING ONLY until actual original reds.
Workload: existing native names fixture, reader64bytes, normal independent
larger-limit writer accepts256byte payload/state or128byte key. Before probe,
assert exact RowLimit sizes. Application exact-signature octet_length returning0
must not change that result. Capture result and DROP probe BEFORE assertions to
preserve independent fault tests, unlike the rejected prior poisoned fixture.
Budget BEFORE expansion: existing names_tests314→<=650lines (prior360budget
explicitly revised for four separate capability owners); reuse existing fixture,
add one private limits entry point, no new test framework/API/runtime/dependency.
Production budget<=20SQLline changes across current page/listing/snapshot/
checkpoint owners ONLY after actual red per owner. No invented SUM/window/int
coverage from octet_length witness; those need their own fault tests. One limits
Rustdoc line correction allowed: current checkpoint accounting48, not stated32.
Falsifier: any original reader delivers the oversized body/key under probe while
healthy control rejects exact size. Reject changes if caller settings are reset,
ordering/checkpoints/bounds weaken, or another function/operator remains able to
violate the same claimed guard. Rollback only this experiment; preserve all reds.

Accounting delivery documentation pre-edit budget: changelog<=5newlines naming
only the actual octet_length collision defect, no blanket SQL-name safety claim.
Update PRD/backlog/progress with protected durability delivery and current narrow
accounting verification; do not expand into SUM/window/integer hypotheses before
this proved slice is delivered. Full native255 is running; preserve product source.
# PG9 patched wake dependency — pre-edit, 2026-10-19

Threat: root Cargo.lock selects event-listener5.4.1, which RustSec0221 identifies
as unsound; deny.toml suppresses that exact advisory. The two fuzz locks contain
no event-listener. Current three locks contain no time/refinery, so the old
time0009 audit exception also needs an unwaived check rather than retention.

Experiment: preserve the existing dependency owner and use its compatible5.4.2
patch. Before changing it, run cargo-deny against a temporary copy of the current
policy with its advisory ignore removed; require the actual selected5.4.1 red.
Then raise the workspace minimum to5.4.2, use Cargo's precise update, remove the
resolved deny exception and absent-time audit exceptions, and revalidate the
existing wake concurrency source claims against the exact patched source.

Budget: root Cargo.toml one replacement line; root lock at most20changed lines,
no added packages; deny.toml at most10removed lines; audit.toml at most10removed
lines (delete the empty configuration if no policy remains); flake.nix at most
3removed lines; wake-nostd one comment replacement; changelog at most5added lines
(ordinary Fixed heading, entry and separating blank lines).
No executable Rust, public API, test selection, lint, baseline, dependency family,
framework, runtime, tooling or new test file. Existing research/backlog owners
record evidence. Revert the dependency/policy slice if unsupported, if the narrow
update adds unrelated packages, or if wake/native/no_std/complete gates regress.

Proof: inspect exact upstream license/MSRV and listen/notify fences; compare
root lock graph before/after; run unwaived current cargo-deny and cargo-audit over
all owned locks, actual wake tests, complete native Postgres suite, and unchanged
signed full Nix gate (including bare-metal wake). Three independent reviews after
every actual test run; compile checks and cached gates get no native-test credit.
PR396 remains frozen and gets immediate delivery attention when its CI completes.

Pre-edit budget refinement from actual Cargo/hakari evidence:5.4.2 drops its
concurrent-queue edge. The supported `cargo hakari generate --diff` returns1
because two generated workspace-hack edges are now redundant, not because a
test failed. Add only the required generated-manifest removal (two lines) and
the matching existing workspace-hack lock edges to this slice. No hand-written
dependency replacement or new package. Keep the <=20line lock budget; inspect
the actual final graph before accepting it. Required generated owners must stay
current rather than retaining redundant dependencies to meet an estimate.
# PG4 bounded migration metadata — pre-edit, 2026-10-19

Threat: runtime schema validation fetches every migration row and each complete
checksum before rejecting unknown versions or mismatched digests. A corrupted or
unsupported metadata table can therefore cause unbounded startup allocation even
though the binary embeds only five migrations. Current source finding, NOT yet
a reproduced defect. Ownership stays in Postgres schema.rs and migration tests.

Before editing production, add two real database allocation laws using the
existing support/memory.rs allocator: a correct checksum prefix plus32MiB tail,
and100,000 unknown metadata rows. Require Schema rejection, unchanged native
metadata, restoration before assertions and successful reopening after repair.
Measure incremental peak allocation across actual from_pool; budget2MiB for
startup, independently of malformed metadata length/count. Original code must
fail the allocation oracle. Do not confuse SQL fixture generation with a bounded
store read, or a returned rejection with a memory bound.

Candidate minimal fix: SQLx QueryBuilder and native pg_catalog.substr, returning
only one byte beyond the longest embedded checksum, and at most one row beyond
the embedded migration count. Compute both bounds from MIGRATIONS with checked
addition, preserving exact equality/length/version/success checks. No hardcoded
checksum length or migration count; no new protocol state, framework, runtime or
dependency. Compare supported SQLx streaming; plain streaming alone leaves the
server result unbounded and must close the connection after rejection, while
native projection/LIMIT cap the response itself. Do not add a custom cursor.

Budget: existing schema.rs at most40added/10removed production lines, at most one
private metadata helper if required by the80line function lint; zero public API.
Existing migration_tests.rs696lines may grow by at most180lines (max876), including
allocator imports/two native laws and an ordinary private measurement helper only
if needed. Each function stays<=80lines. Changelog<=5lines, README<=5lines. Existing
research/backlog owners record evidence. Roll back this slice if exact checksums,
unknown-version rejection, runtime privileges or migration semantics regress.

After actual original reds, run targeted/relevant complete native cases, independent
wrong-limit and truncated-prefix controls, strict lint and unchanged full signed
Nix gate. Three contract/execution/assertion reviews after EACH actual test run.
Current dependency signed gate28513 remains frozen; do not run another native or
build job until that handle is terminal. Deliver its focused PR promptly oncegreen.

Scheduling refinement BEFORE execution: actual live nextest derivation azbj8qv7
has neither DATABASE_URL nor impureEnvVars, and uses its private Nix target tree.
Its adapter guards therefore make no native database calls. One dedicated native
suite can characterize the two metadata laws using the existing shared developer
target and PG17 cluster, with no other native suite running. Allocator counters
are process-local; external CPU load cannot turn32MiB into a2MiB allocation pass.
This supersedes the conservative all-builds wait above only for this independent
characterization. Keep ONE full Nix gate and ONE actual native suite, never two
native suites or shared-target builds. Do not infer comparable performance from
timings under concurrent host load. Dependency source/index remain frozen.

## PG8 supported operational deadlines — 2026-10-19

Threatened invariant: a caller-configured SQLx acquisition deadline and native
Postgres statement/lock deadlines must bound actual adapter work without turning
an interrupted COMMIT into a safe retry. Existing cancellation tests do not prove
these configured deadlines, and permanent server loss must permit bounded close.
This is explicit PG8 operations work, not further security investigation.

Workload: real blocked append/read/startup, post-insert sleeping trigger, exhausted
two-connection pool with observed listener readiness, and private-server shutdown
with repeated failed operations and bounded release. Use owning SQLx0.9 options,
Postgres native timeouts, existing Tokio/cluster facilities; add no deadline API.
Falsifier: exact expected native error absent, watchdog expires, any failed
pre-COMMIT write/key lands, leases remain blocked after cleanup, or close hangs.
Wrong-option controls disable the selected timeout and must fail the same oracle.
These characterize supported controls: no assertion that original production must
fail a newly invented timeout policy. Existing post-COMMIT uncertainty laws stay.

Change budget BEFORE source/test edits: zero production Rust/dependency/migration
changes; one adapter-owned deadline_tests.rs ≤380lines, at most five native laws
and four private fixture helpers; existing README ≤45added lines and changelog
≤3. Keep each function≤80lines/5args and all existing lints/selection unchanged.
Rollback boundary: only this test/doc slice. Exact configs/numbers are test
fixtures, not deployment SLOs. Native characterization may coexist with the
independent host Nix gate, which has no database env and a private build target;
only one actual native suite at a time, and no timing comparison as capacity.

## PG8 bounded operational diagnostics — 2026-10-19

Threatened invariant: operators must distinguish listener readiness/retry,
notification-independent rescans, append conflicts, commit uncertainty and scan
work without logging payloads/credentials or creating per-stream metric labels.
Current listener discards session errors and recv hides supported reconnect
signals; current page/wake owners have no adapter tracing. This is explicit PG8
acceptance, not additional security investigation.

Reuse workspace tracing0.1.44/tracing-subscriber0.3.23 and existing store spans.
Add an optional adapter tracing feature (no new dependency package); fixed event
names/status labels, numeric rows/cutoff values, skip_all instrumentation. Use
SQLx0.9 try_recv's documented reconnect signal instead of duplicating its retry
state. Preserve all timer/read/commit semantics, including unknown completion.
Native characterization must first show the existing production emits no adapter
readiness/page/fallback/commit evidence using the same collector. Falsifier:
missing/misclassified event, changing error/outcome/ordering, or logging actual
payload/key/password probe bytes. Controls remove a meaningful event and must
fail its required observation, not poison the native fixture.

Change budget before code: one new diagnostics_tests.rs <=260lines and <=four
private collector/fixture helpers; existing production files store/wake/error/
durability and optional write owners <=160added/30removed lines total, at most
one private helper if existing function length requires it. Manifest/lock feature
edges <=12lines; generated Hakari only if supported generation proves necessary.
README <=45lines/changelog<=3. No public Rust type/API, migration, runtime,
framework, tooling, lint/threshold/test-selection change. One independently
reviewable diagnostics slice, isolated from frozen metadata and deadline WIP.
## PG8 sustained native workload — 2026-10-19

Threatened invariant: sustained mixed writes, projection restarts and listener
reconnects must not lose/duplicate events, mix saved state/position, accumulate
client memory/connections, or need a later notification to advance. This is the
explicit 30-minute PG8 soak, not a substitute for the separate capacity matrix.

Workload before implementation: private real Postgres cluster, existing SQLx/
Tokio/cluster/allocation facilities; 16 competing producers per round, alternating
one/eight-event batches, and16 saved-state consumers reconstructed from native
checkpoints each round. Each minute replace the reader, terminate its listener,
observe actual reconnection, commit one native event without NOTIFY, require
first-event recovery within the established3s test bound with no later writes,
then rebuild a full-history reader. Hold page limits128rows/1MiB and pool limits
24+24+4. Zero unknown write results may be retried; any unexpected error fails.

Acceptance: exact one winner/15conflicts per round, strictly increasing global
positions/payload ordinals, exact persisted consumer pairs and final event count,
minimum1800s actual steady work, >=30fault/restart cycles; peak incremental
allocation <=4*16*1MiB+8MiB, retained <=baseline+8MiB after closing pools,
zero remaining producer/consumer backend sessions within3s. Report actual warmup,
elapsed, counters, allocated bytes, resident allocator bytes and connections.
RSS and tail latency remain separate capacity evidence. Short explicit runs get
smoke credit only, never30-minute acceptance. Run the full1800s after integrating
the final diagnostics parent and after other local full gates finish.

Falsifiers/controls: disable the one-second rescan in an isolated source control;
the native no-notification phase must time out. Corrupt an event ordinal control
must fail actual fold validation. No assertion/budget/timeout relaxations.

Change budget before code: one standalone Cargo benchmark<=500lines/15private
helpers using the existing Fjall reporting-benchmark pattern; manifest<=6lines,
README<=14lines; zero production Rust/API/migration/dependency/tool/framework/
runtime changes. Reuse tests/support/{cluster,memory}.rs unchanged. Rollback only
this reporting benchmark/doc slice; preserve all source and native evidence.

## Soak configured exclusion-lock rejection — 2026-10-19

Second native long run fails after22faults at exact SQLx55P03 during key exclusion
checking, under the preregistered1s lock_timeout. A lock deadline is an explicit
pre-COMMIT rejection, not an optimistic Conflict or successful commit. Preserve
that distinction and all unknown outcomes. Do not increase deadlines or retry.

Before fixture policy changes, characterize a real uncommitted competing key
under the existing deadline owner: exact Sqlx55P03, independent zero event/key
inventory after blocker rollback, and healthy append after settled rejection.
Pair with existing after-COMMIT SQLSTATE tests:55P03 there must stay Unknown.
This is native operational contract evidence, not a new security investigation.

Budget: existing deadline_tests.rs <=380total lines (current297), add one <=75line
law using existing four helpers; zero production/API/dependency changes. Then
allow only the exact typed pre-COMMIT rejection as a separately reported soak
counter, never Conflict. Keep one exact winner and all remaining requests either
Conflict or that known rejection; exact consumer/global counts and every original
3s/5s/1s/10s/1800s/resource/fault bound unchanged. Keep all reds and record failure
rates; no throughput guarantee is invented. Unknown/other causes remain fatal.
One private counter product may replace the existing model/report scalars to
keep one owning source for round/event/fault/rejection counts, within500benchlines.

## PG9 supported feature verification — 2026-10-19

Existing Python wrapper performs single-package library checks while retaining
all dev dependencies; the known Cargo feature-unification problem means this is
not isolated-library proof. It also omits Postgres from the CI matrix. Existing
native archive enables all adapter features, so its seven smoke laws do not
establish the individual-feature graphs. Remove the wrapper rather than porting
its machinery to another project tool.

Use locked Nixpkgs cargo-hack0.6.45 (MIT/Apache-2.0), supported --each-feature and
--no-dev-deps, in disposable CI checkouts only: the tool temporarily rewrites
manifests. Verify exact options/lock behavior before retaining. Keep original
three library unit-test modes and add Postgres; unit harness feature unification
is explicitly not isolated-library credit. Expand existing SQLx/nextest archive
factory for default and each five adapter features; run each complete selected
native suite through the existing Linux Postgres integration hook, preserving
all-feature suite and mandatory smoke laws. No new runner/framework/custom tool.

Budget before edits: delete45line scripts/check-features.py; flake.nix<=65added/
10removed lines reusing archive/integration owners and existing devShell, checks.yml
<=25added/3removed lines. No productionRust/publicAPI/lock/runtime/framework/new
project scripts, lint or threshold changes. Existing fuzz Python parser will use
Cargo's locked metadata+jq,<=8added/9removed lines; jq and cargo-hack Nix-owned.
Falsifiers: dev-only dependency masks a required feature; Postgres lacks a listed
feature's actual native suite; empty/guarded job passes; parser changes locked
engine version; ignored library/unit/DB failure. Preserve input locks and public
capabilities. Record every exact run and three reviews. Native local execution
and full Nix gate wait for the active soak to finish.

## PG9 AFL saved-crash verdict — 2026-10-19

Threatened invariant: required fuzz-gate must reject saved AFL crashes even when
the bounded engine exits zero, and a failing producer must survive tee. Existing
AFL job uploads on always() but has no crash-file verdict or explicit pipefail.
First characterize those shell verdicts against missing/README-only/crashing
artifact directories and nonzero piped producer. Then add one inline workflow
verdict, retain existing CMPLOG, duration, target set, seed/corpus and uploads.
A real-engine saved-crash/zero-exit control is separately required before claiming
that end-to-end boundary; constructed artifacts prove only the gate itself.

Change budget: fuzz.yml <=20added/5removed lines, no production Rust, harness,
dependencies, custom tool/test framework, runtime, lint or threshold changes.
Use supported Bash glob/file predicates and pipefail, not a new script. Keep
crashes uploaded on always(); don't mistake AFL's informational README for input.
No other PG8 capacity or security work is bundled. Native-only checks wait for
full soak; shell checks may run independently.

PG9 public index correction budget before edits: rootREADME<=8added lines and
adapterREADME<=4added/2removed lines, no Rust/runtime/tooling changes. List the
already-implemented Postgres adapter beside Fjall, link its existing concise
verified-connection example, and replace obsolete0.1 quickstart dependency with
the independently published0.5 series. This is documentation of the current
public surface, not a production-readiness claim before remaining acceptance.

2026-10-20 integration budget clarification before final gate: fuzz.yml net
16added/8removed includes deleting the eight-line project Python metadata reader;
previous removal estimate5 omitted that deletion. Bound now20added/10removed,
no new runner or production logic. Native factory58added/19removed remains
within65/20; workflows retain every original target, timeout and artifact rule.
PG9 real AFL saved-crash control, preregistered2026-10-20 before edits.
Threatened invariant: AFL engine may save crashes and return success; required
fuzz gate must fail and preserve the actual crash artifact.
Use a never-merged proof branch based on the exact ordinary PG9 candidate.
Only its existing fuzz workflow gets one temporary step before AFL build, for
value_event_type: replace that job's checked-out harness with an intentional
panic on every input except initial b"A", create a dedicated one-file A seed,
and select that dedicated input for this one target. Other targets, engines,
CMPLOG, time bound, tee/pipefail, saved-crash verdict and unconditional upload
remain identical. No checked-in production or harness source modification.
Dispatch existing fuzz.yml with duration10 through ordinary signed commit/hook.
Expected result: initial seed calibrates; real mutation causes native panic;
Fuzz with AFL++ step succeeds (zero engine exit) with actual saved file(s);
Reject saved crashes fails; Upload crashes succeeds; aggregate fuzz-gate fails.
Download artifact and inspect regular non-README crash content. Read exact step
conclusions and engine counts/time/CMPLOG, not merely aggregate red. Apply three
reviews: owning gate contract, actual engine/crash/zero-exit oracle, unchanged
candidate verdict and thresholds/source outside isolated intentional injection.
Rollback boundary: never merge proof branch; main candidate contains only the
normal verdict. If engine exits nonzero/calibration fails/no real crash, record
the control as invalid and fix the fixture before claiming proof. No constructed
file can substitute for actual engine output. Budget temporaryworkflow<=30lines,
zero production/API/dependency changes, no framework/runner/tool introduced.

## PG9 actual AFL crash artifact repair — 2026-10-20

Real controlled engine run39450659063/job118724284510 uses CMPLOG, calibratesA,
runs10s, saves3real crashes and returns0. Candidate saved-crash verdict fails1
as required. Existing unconditional actions/upload-artifact then rejects native
AFL filenames containing colon; no artifact is retained. This is a real workflow
failure, not a successful complete control. Preserve log/first red.
Before editing: package existing out/*/crashes directories with standard tar/gzip
under a portable archive filename, upload it through the existing always() rule.
Keep original crash paths/bytes inside archive and worker identity, preserve all
engines/durations/CMPLOG/tee/verdict/corpus behavior. No new runner/dependency/tool.
Budget normalfuzz workflow<=28added/12removed total (previous20/10 omitted the
newly demonstrated packaging requirement), one <=9line packaging step plus
artifact path; no production/harness/API change. Repeat the same real10s
controlled job and require engine0, actualsavedfiles, verdict1, uploaded archive,
inspected original crash bytes and aggregatefailed. Clean campaign must still
pass with no archive; absent data is not a catch. Normal signed hook unchanged.

Local sequencing refinement: capacity native measurements and its1594case
regression run are terminal/passing; only final cross-target compilation is
active, with shared Nix database contention observed. Permit ordinary cached
hooks for this workflow-only artifact repair while that tail finishes; no new
native/performance workload or duplicate runtime suite. Both unchanged hooks
still enforce every check. Source-filter/gate/assertion changes are prohibited.
The previous one-full-runtime-gate limit remains for actual new test executions.

## PG8 measured capacity matrix — 2026-10-19

Threatened invariant: bounded pages must stay bounded with larger histories and
many consumers, while read/append service and resource saturation are measured
at the same durable database settings. Existing correctness timings and the
mixed soak do not supply latency distributions, noise or a capacity matrix.

Before implementation: reuse native Cargo reporting-benchmark pattern and the
unchanged Postgres cluster/allocation fixtures. No framework, dependency package,
public API, production SQL or runtime change. Prepare actual adapter histories
10k/100k/1m, short100-event streams and one long stream,32/4096-byte payloads
matching prior read baselines. Fixed128row/1MiB pages, fixed pool cap; record actual
native settings/hardware/toolchain and fixture sizes. Seed/warmup outside timing.

Measure1/16/128 simultaneous consumers at early and late resume positions, both
stream/global paths. Report construction-to-first-page service distributions,
p50/p95/p99, operation/event throughput, sample count, repeated-window dispersion,
client allocated bytes/peak/retained allocator memory, process RSS, pool/backend
counts and query plans. Also measure complete-history drain explicitly; distinguish
full rebuild from bounded page service. Append profiles cover one/eight-event
batches and competing versus independent writers, recording exact conflicts,
commits and unknown results. No retry of uncertain writes. Repeat unchanged
profiles to establish a noise floor before tuning or comparison. No invented
throughput/latency SLO: publish measured local deployment limits and limitations.

Falsifier: payload/order/count/resume mismatch, growing retained cursor memory
at fixed pages, resource bound exceeded, missing workload axis, unexecuted or
insufficient samples misreported as quantiles, or repeatable regression dismissed
without an equal-durability comparison. Retain raw results and source/version;
measurement is not a green correctness assertion or a production fleet guarantee.

Budget before code: one native Cargo reporting benchmark<=700Rust lines,
manifest<=6lines, README<=18lines; no production Rust/API/schema/dependency/tool
changes. Reuse support/{cluster,memory}.rs unchanged; standard Rust statistics,
SQLx/Tokio and native Postgres/OS statistics only. If full target workloads exceed
this host's storage/time capacity, record actual boundaries and continue to the
required adequate execution environment rather than dropping or relabeling cases.
Only ONE native workload at once; full capacity measurements wait for the full
soak and local Nix gates to finish. Preparation can proceed without running it.

Capacity accounting clarification before edits: allocation traffic bytes and
allocation call count are different measured facts. Extend the existing shared
memory fixture by<=4lines with one successful-allocation counter, preserving
all original LIVE/PEAK/TOTAL semantics. Do not add a second allocator. Report
its defined counting boundary and measurement overhead; this is a native
measurement fixture, not a new production state or testing framework.

## Capacity plan-mode evidence correction — 2026-10-20

Actual full-matrix diagnostics force generic plans. Million-row initial scans
show native sequential scans (~80ms for one32B diagnostic), while the timed
ordinary auto-mode read windows are~0.4–0.7ms at one consumer. These are different
plan modes and must not be conflated or called a timed-path regression.
Before retaining evidence, record native auto plans as well as the separate
forced-generic stress plans, plus selected session's actual generic/custom plan
use counters before replay. Keep every timed workload/validation/timeout/row/
byte/pool/sample/window setting unchanged. No library optimization justified by
this forced-mode diagnostic alone; preserve the original full baseline.
Budget remains700benchmark lines/strict80function lines; one additional native
mode loop and counters in existing plan observer, no new helper/API/tool/frame-
work/runtime or production change. Falsifier: absent/mislabeled ordinary plans,
wrong sample selection or source settings/assertions change to make a result pass.

Capacity documentation budget before final record: add one feature-local
capacity.md<=190lines; README<=20added (two links beyond original18estimate).
Retain every raw measurement, differentiate generic diagnostics from ordinary
latency and record variation/saturation. No production optimization is supported
by these same-product baseline/observer results; final gate checks benchmark
compilation and original allocator owners. Actual model runs already passed.

Final documentation minimization,2026-10-20: fresh owner-contract read finds
EventExporter blanket documentation incorrectly claims every adapter stream is
concrete/unboxed, while Postgres intentionally uses bounded BoxStream. An adjacent
StreamLister implementation example still says Postgres SELECT DISTINCT, whereas
its retained implementation pages native keys with EXISTS. Correct/remove these
two implementation claims in their existing owner, not behavior or interface.
Budget BEFORE edit: crates/store/src/export.rs<=4added/7removed comment-only lines;
zero executable Rust/public API/dependency/schema/test changes. Falsifier: doc
diff changes behavior, weakens bounded-memory/view guarantees, or leaves a false
universal allocation promise. No new mirror test for prose; normal doc/format/
full signed gate remains. First source-audit pass found these minor doc issues,
so it is not counted as one of the two final clean audits.

Final report accuracy also removes the legacy AFL upload comment that its crash
directory exists only after a crash. The real corrected control's other clean
jobs produce empty-directory archives, so artifact presence alone is not a crash.
Budget before comment edit: existing fuzz.yml<=2added/4removed comment-only lines,
no change to archive/verdict/always()/inputs/engines. Existing live clean/control
evidence already establishes behavior; no new mirror test or runtime rerun.

Final delivery grouping refinement (2026-10-20): retain the reviewed capacity
benchmark budget unchanged (690/700 lines; manifest4/6; README19/20), and include
the already preregistered comment-only minimization plus final acceptance/status
records in the same completion PR407. This avoids a separate source-comment PR
without combining executable behavior changes. Additional production budget is
export.rs2added/4removed comments; workflow2added/4removed comments. No new API,
dependency, runtime or test assertion. Existing complete source audits remain
valid only while the production fingerprint stays dc45f0ac1d4420a4eca4cfcb17f5e2e2bd7000e3857dca8b6ebacf2a40954ae2.
Normal signed gate/current-head protected CI still required; publication and
exact registry consumer remain separate observable delivery requirements.

Final publication record budget (2026-10-20): eight documentation/research owners
and at most six evidence artifacts; no production, test, manifest, lock or
workflow change. Revise the initial 1000-line estimate to 1400 added record and
evidence lines before editing: the actual selected CI transcript is267lines,
registry compile207lines, native39lines, and the independently executed consumer
source105lines, before identity/publication receipts and required three-review
records. Preserve these receipts rather than reduce verification. Delivery.md
remains at most100lines. Normal signed hook and protected CI remain required;
cached output gives no additional runtime credit.

## CP planning — 2026-10-21

User-authorized scope: write a researched PRD on a new branch for moving Labcast's
controller into Mnesis as `mnesis-command::CommandProcessor`. No product code,
Labcast files, dependencies, workflows, publication or persistent goal changes.
Branch `docs/command-prd`, base `27de3b4a4aad59909ffaccdf6fa83e8b256ccca6`.
Planning budget before edits: at most 16 documentation/evidence files, 1,200 new
handwritten lines, plus unchanged-source test logs and input fingerprints.
Owners: docs/prds/command.md, docs/prds/command/research.md and evidence/, existing
backlog README/status and research HYPOTHESES/SOURCES/EXPERIMENTS/PROGRESS.
Hypothesis: existing generic processing can move without Labcast policy or a new
framework, while closing actual compatibility, outcome, consumption and test gaps.
Falsifiers: copied private persistence rules in consumers; lost retry identity or
uncertainty; runtime dependency leakage into core; historical tests reported as
current integration; arbitrary capacity/latency claims; omitted source provenance.
Validation: current source/locked-owner review, primary-source terminology and
lifecycle research, unchanged Labcast tests and existing optimized measurement,
three post-run reviews, two fresh PRD acceptance/dependency/scope audits and diff/
local-link checks. Whole-product delivery remains future CP acceptance work.

Source-location correction, 2026-10-21: the user moved the source to
`/Users/joel/Code/labcast/controller-archive`, not `labcast-server`. All 30
baseline input hashes match there. Future consumption belongs in the empty
server workspace; mobile is excluded. Preserve historical execution paths and
the archive. No downstream mutation or workflow redesign follows from the move.

## Tower actor fit experiment — 2026-10-21

User requests executable qualification of tower-actor. Scope: disposable Cargo
consumer under /tmp, existing Nix environment, original archived engine/model
copied byte-for-byte, no production or dependency changes in either repository.
Budget: two probe source files plus unchanged engine/model, <=500 authored Rust
lines, <=150 research lines and one compressed evidence bundle. Preserve the
existing PRD draft. Contract: bounded admission, serial execution, post-admission
caller cancellation does not cancel work, original receipt survives worker loss,
typed causes and observable shutdown. Falsifiers: lost admitted request, reordered
writes, erased worker cause, or missing task-completion evidence. Compare published
tower-actor 0.2.0 against existing host behavior; record gaps without weakening tests.
Three distinct case/source/execution reviews follow every actual test run. No
performance improvement or full Postgres integration claim from this experiment.

## Tower service and observability requirements — 2026-10-21

User now requires Tower Service, tracing and telemetry in the PRD. Documentation
only. Amend planning budget to <=16 files and <=1,450 handwritten added lines
(earlier 1,200) for explicit interface/lifecycle and observable instrumentation
acceptance; existing evidence bundles remain separate. No dependency/code changes.
Use existing Tower 0.5.3 and tracing 0.1.44 conventions; qualify one optional metrics
facade and application-owned OpenTelemetry bridge. Falsifiers: changed retry or
admission semantics, lost request context, miscounted committed commands, unbounded
metric labels, exporter ownership inside the library, or a second work queue.

## CP0 implementation authorization and budget — 2026-10-21

The user now authorizes CP0–CP9 implementation, protected PRs, publication and
server consumption. Persistent goal active, no token budget. Preserve original
planning patches and untracked records in /tmp/mnesis-command-start-20261021.
CP0 budget: eight record owners (CONTRACT/HYPOTHESES/PROGRESS/EXPERIMENTS/SOURCES,
command PRD/research and backlog status), frozen source archive/manifest, and a
disposable Cargo prototype of <=500 authored Rust lines. No production edits.
The archive now contains 2,490 files including generated fuzz corpus; all 30
original baseline hashes still match. Freeze all current files and retain only
qualified canonical seeds in the future product; do not delete source inputs.
Prototype Tower reservations/termination, typed adapter causes and acceptance
reading. Reuse earlier actual baseline runs when source/dependencies match;
repeat only newly introduced or changed boundaries. Source preservation and
permission: the user expressly instructs extraction and publication of their
Labcast source under this project's process; record this authorization and
preserve provenance/attribution. No third-party source claim is inferred.

## CP1a mechanical extraction — 2026-10-21

CP0 qualified; implementation branch feat/command-extract starts from the signed
CP0 specification commit21afe896. Initial source copy/rename is separate from
behavior changes. Budget: original four production modules (937 lines), original
canonical tests/helpers/fixture (1,891 lines), new manifest/README; root workspace
manifest/lock, Hakari owner, release-plz disable entry and architecture docs.
At most150 net production lines beyond baseline across CP1. No normal third-party
package added in this mechanical pass; dependencies align with existing workspace.
Keep package publish=false/release=false until CP8. Do not import the archived
nightly harness or generated corpus into product ownership; frozen source retains
all inputs. Run owning original tests then three distinct reviews; compiler,
format/Clippy and full gate before commit. The following CP1b adds required Tower
Service within the same budget and preserves the failure probes' requirements.
CP1 ownership adjustment before edits: repository clippy.toml prohibits std Mutex.
Use existing workspace parking_lot0.12.5 for short synchronous admission sections
and copied fault fixtures, without holding a guard across awaits. No lint waiver.
Actual root lock diff adds tokio-util0.7.20: CP0 incorrectly called it already
locked (it was in the disposable prototype, not the workspace lock). Correct that
record. One new helper package is qualified for supported PollSender; parking_lot
is already locked. No custom polling framework or new synchronization primitive.

CP1 record correction before edit: update the existing BASELINE and INVENTORY
resume pointers (two documentation owners, <=24 added lines). The command PRD's
feature-local research remains authoritative for measured results and sources;
these root records currently end in historical Postgres work. Reuse links rather
than duplicate acceptance tables. No code/test/configuration change or new tool.

Follow-up contract audit for CP2/CP5 (not a completion claim): current native join
preserves panic payload but cannot return Shutdown on panic/abort. The new CP1
panic test proves cause73 plus uncertain reply identity, not unresolved-request
handoff through failed completion. CP5 must reconcile the full completion/custody
requirement with abrupt-loss caller obligations and test panic during post-append
fold plus failed reconstruction. Do not treat a handler panic before append as
proof of the post-commit path. Also review delayed fault tasks: Gate::done observes
append completion but is not a native task join. Qualify teardown at its actual
boundary before asserting zero live task resources. No source changes in CP1.

CP5 follow-up: admission currently holds its parking_lot gate while PollSender
poll_reserve/close or a failed try_send can wake/drop caller-owned objects. Prove
that a caller Waker or command destructor reentering stop cannot deadlock. These
are distinct from the Tokio-owned worker wake. Readiness itself does not enqueue,
so its check/poll may be separable from the enqueue/stop linearization gate.
Use a bounded real-SUT witness before any change; no custom queue/waker runtime
or gate weakening. This remains a hypothesis, not a reproduced defect.

## CP2a: typed causes through the owned worker — pre-edit budget

Threatened invariant: adapters/kernel/domain causes must survive every reply,
including uncertain exact retries, without downcasting or recursive error chains.
Carry S::Error as the handle/channel's trailing error parameter; keep start/resume
inference through a factory returning the concrete typed handle. Use one public
Cause union for definite failure and optional uncertain cause. The pure adapted
aggregate retains a private decision error; promote it once at the engine edge.
StoreError::Kernel after save remains uncertain; ExecuteError::Kernel before save
is definite unless older pending custody exists. Keep pending until valid receipt
observation. Preserve format, queue order, request identity, limits and runtime.
Budget: four src owners and recovery/adaptation/history/model tests (eight files),
<=250 net production lines, <=250 test lines, one new public Cause type. No new
dependency/configuration/schema/runtime/tool. Resource/codec distinctions are the
next CP2 slice. Falsifier: typed leaf cannot be matched externally, any previous
pending request disappears on failed retry/read, a post-append failure becomes
retry-safe, or original tests/workloads weaken. First replace the canonical lost-
response test's downcast with typed matching; preserve original row/receipt laws.
CP2a test budget refinement before adding missing acceptance witnesses: <=350
new/changed test lines instead of250, still below the PRD's500-line ceiling.
Keep the same eight owners and250-net-production budget. Add actual committed
append followed by application fold panic, failed fresh reconstruction, healthy
exact recovery with no second handler call, and a real over-limit history whose
kernel source is matched through the public typed error. No synthetic repository
or weakened cleanup/selection/assertions.
CP2 scope refinement before further edits: finish codec/resource diagnostics in
this same focused typed-error change, keeping one coherent public failure model.
The same eight source/test owners, <=250 net production and <=350 changed test
lines remain; public additions become Cause plus CodecError (two, within the PRD
ceiling). No dependency/format/runtime change. Distinguish request/record bytes,
lossy serialization, malformed/unsupported records, queue fullness/invalid limits,
history exhaustion, domain replay limit and version overflow. Retain original
codec sources where they exist; replace fabricated JSON errors for own invariants
with typed variants. Compute fact totals once with checked arithmetic. Constant
nonzero limits must be checked during compilation, with no runtime fallback.
Falsifiers: changed accepted bytes, new false retry-safe outcome, old receipt
unreadable at capacity, lost source, changed workload or >budget. Preserve all
original tests and add/strengthen the owning diagnostic assertions before the
full gate. Documentation/evidence/status updates remain mandatory project records.
CP5 source audit follow-ups, not yet reproduced: the worker's biased select can
poll stop pending, then dequeue a call and synchronously wake a waiting sender
whose Waker calls stop. Check whether queued work then executes after stop was
published. Force this with the existing delayed-active Gate, one queued call and
a third readiness waiter's standard Wake implementation; no scheduler luck or
production-only test hook. Also retain the recorded reentrant-wake/drop risk
while Admission is locked. Test before selecting the smallest locking change.
For failed-completion custody, compare supported Tokio join/abort facilities;
a typed completion owner over the original join and one shared authoritative
pending request may be necessary. Do not copy pending state or discard native
terminal causes. Any public-surface budget extension needs this concrete proof.

## CP3: force both acceptance contenders past the same head — pre-edit budget

Threatened invariant: join! alone can pass when one host finishes before the other
loads, so it does not prove CAS arbitration or duplicate/mismatch behavior after
both handlers ran. Extend the existing FaultStore with one optional append gate,
captured before invoking the actual inner append. Share one canonical race helper
between recovery and generated histories; one isolated aggregate counts handler
calls, without adding observer data to commands/persisted facts. Force both append
entries before any release, observe zero rows/two handler calls and both replies
pending. Release a selected first arrival then its loser; check exact whole stored
record, original receipt, state, duplicate/conflict/mismatch and no redecision on
retries. Exercise both winner orders and ID/body boundaries including1/u64MAX and
0/u8MAX; preserve existing256 serial histories and add256 controlled schedules.
Budget: faults/recovery/properties plus one support/races.rs (four test owners),
<=350 net test lines, zero product/API/dependency/runtime/config/format changes,
plus required records. No competing store/runtime. Falsifier: either contender
has not reached real append, any row lands before both are held, duplicate rows,
changed original identity/receipt, mismatched handler accounting or incomplete
observed worker teardown. First demonstrate the old join-only case has no such
barrier assertions, then replace it rather than add a redundant concurrency law.
CP3 fixture ownership refinement before edits: include history.rs as a fifth
small test owner so properties/history/races share the existing faults and shutdown
modules once at the integration-target root. This removes duplicate compilation
of the fixture without exporting any product interface. Same350-net-test ceiling.
CP3 negative-control selection before run: use existing cargo-mutants to delete
Engine::submit's Err(Conflict) reconciliation arm and replace State::receipt's
original==request guard with true. Run the real canonical concurrent case only.
Both mutations should reach successful controlled append, then fail original
receipt/mismatch assertions, rather than fail fixture setup. Do not count a
build failure or gate-entry timeout as proof of this concurrency law. This is a
targeted falsifier; full current-head CI mutation coverage remains separate.
The maintained generator does not offer deleting that conflict arm (list produced
only the identity guard, zero tests). Before execution, select its equivalent
reachable falsifier: change Engine::write's confirmed-conflict classifier guard
to false. The real CAS then returns an unknown outcome instead of reconciling the
original receipt after both contenders append. Keep the identity guard=true
control. Both must fail actual response assertions after real append, not setup.
CP4 audit reminder before implementation: a stateless acceptance codec can validate
one record's metadata but cannot prove cross-record command-ID uniqueness or order.
The current adapted replay owner checks those stream laws. Keep that distinction
explicit when exposing the public reader and writing its projection witness;
do not claim row decoding alone validates a whole history, and do not duplicate
the private schema or invent per-fact physical positions/checkpoints.
CP3 observed gate prerequisite before edit: an unchanged macro diagnostic test's
nested rustc aborts134 because dyld resolves its hard-linked executable under
/nix/store/.links and cannot find @rpath/librustc_driver. The actual driver exists
under the selected toolchain/lib, same inode compiler; forced direct alias probe
fails134 without fallback and passes0 with that exact toolchain/lib. Budget
extension: one supported Darwin-only nextest environment fact in flake.nix (<=7
lines) plus required evidence. Use lib.makeLibraryPath [rustToolchain] with Cargo's
supported DYLD_FALLBACK_LIBRARY_PATH, no new derivation/tool/wrapper/runtime and
no test/filter/lint/workload change. Scope it to the failing nested-compiler check;
Linux evaluation/behavior stays unchanged. Retry the entire original1629 suite.
Falsifier: alias still fails, Cargo discards fallback, another platform changes,
any tests skipped/softened, or full normal gate fails. The initial gate was already
failing and was agent-interrupted when free space reached581MiB; preserve that
failure and exact completed/unfinished counts, not partial green credit.
## CI blocker: current rustls-webpki patch — 2026-10-21

PR411's live cargo-deny job fails RUSTSEC-2026-0382 on rustls-webpki0.103.15.
This is an observed delivery blocker, not an expanded audit. Primary advisory
issued2026-10-20 selects fixed0.103.16. Isolated fix/command-ci worktree from main;
budget root Cargo.lock and four existing research records only, <=100 authored
record lines. Existing fuzz locks do not contain this package. Update precisely
this patch, keep all checks/advisories blocking, normal gate/PR/protected merge.
Preserve command extraction and unrelated releasePR410.
CI-fix budget extension before editing: add one supported Crane setting in
flake.nix, because nextest tests passed1594/1594 but archive installation failed
ENOSPC. cargoNextest defaults doInstallCargoArtifacts=true; no project consumer
reads its target.tar.zst. Keep coverage output, every test command/filter and
all checks unchanged. Set this flag false only for the check-only nextest owner.
Do not alter buildDepsOnly artifacts or the native Postgres executable archive.
Raw failed gate retains exact test results and disk failure. This is an unused
build-output reduction using Crane's supported API, not reduced verification.

CP1 current-head mutation repair, before edit: CI169 mutants reports two genuine
public-capacity coverage misses, one nonterminating call predicate and one
private-State Debug rendering miss (three missed total, one timeout). Budget:
existing recovery/model tests <=25 added lines, existing .cargo/mutants.toml <=6
policy-registration lines, and owning records; zero production/dependency/API/
format change. Exercise advertised capacity in its canonical exhaustion scenario;
bound the existing worker-exit reply waits so the wrong call predicate fails an
assertion rather than only the runner. Private State never escapes the crate's
public API; its Debug rendering is incidental under the repository's existing
diagnostic-only exclusion policy. Register only that exact private implementation;
exclude no capacity, lifecycle or other behavioral mutant. Preserve failure and
actual artifact counts; review the runtime catches/unviable cases before delivery.
CP2 parent-integration gate prerequisite, before edit: reuse the proven five-line
Darwin nextest fallback from the CP3 gate. This same repository toolchain failure
can affect the updated typed-error gate; actual hard-link probe fails134 then
passes0 with selected driver, and the unchanged macro test/full1629 suite now pass.
Budget extension: one Nix owner (five lines) beyond eight source/test owners;
zero additional product/API/dependency/format changes. Same supported setting,
same owner, no duplicated configuration definition, wrapper or framework.
Keep it identical across the pending branches so protected integration converges.
CP3 parent integration preregistered in primary before the merge: preserve every
forced race and imported stronger parent assertion, exact private Debug policy
and both research histories. Product source must remain identical to CP2.
No new product owner/API/package/format/workload/gate change; final combined full
gate and normal signed hook are required. Following protected main integration
must preserve the exact reviewed source and uses ordinary Git/protected workflow.
CP3 current-head CI repair before edits: run39649396688/job119393433782 fails the
canonical forced race at first winning receipt with Uncertain after2s. Gate::wait
notifies entered BEFORE polling the shared release semaphore; entered-first need
not enqueue-first on the two-thread runtime. Prove this permitted ordering using
a disposable test-only hook: pause the first gate waiter until the second has
actually polled its semaphore acquire; use poll_fn and a registration signal,
not sleeps or longer deadlines. Run the actual canonical race. Preserve its red.
Then use independent append gates per contender over two FaultStore wrappers
sharing the SAME actual InMemoryStore; explicitly release the selected host's
gate. Keep all192fixed/256concurrent/256serial schedules, both winner orders,
response/raw-row/state/handler/teardown assertions and2s bounds unchanged. Reuse
FaultStore with a shared-Arc constructor, no competing store/framework/runtime.
Budget: existing faults and races test owners, <=100 net test lines (cumulative
<=350), <=35 disposable probe lines, zero product/API/format/dependency/config
change plus records. Falsifier: forced original ordering does not reproduce,
candidate still times out, stores differ, any contender can steal another's
release, reduced workload/deadline or wrong first-failure assertion. Remove the
probe after red/green evidence; normal final gates and current-head CI required.

## CP4: validated public records and checkpointed fact consumption — pre-edit budget

Threatened invariant: callers currently cannot read accepted facts through the
single codec owner; copying private JSON or flattening facts into fabricated
positions would break validation/checkpoint semantics. Retain private Serde Record
and Codec; expose opaque Acceptance plus read-only AcceptanceCodec, delegating to
that same validation. Implement within model.rs to borrow existing fields without
extra forwarding getters or widened construction rights. Public generic order
C,E,I=NonZeroU64 lets ordinary consumers name command/fact only; CP0's disposable
C,I,E prototype was never a published contract. DomainEvent/Message delegation
lets existing decoded streams/Projection checkpoint a whole record; no new runner.
Do not derive Serde on the public opaque result or require fact Serialize/Clone
for reading. Document per-record checks versus cross-record replay validation.

Budget: model/lib, manifest/lock as needed for existing store projection dev
feature, owning model/adaptation/reader tests and at most two fixtures (<=8 code/
fixture owners; <=250 net product lines, <=500 test lines; two root types, total10).
No new normal package, framework/runtime/development tool/Python/schema. Existing
store projection feature belongs in the dev dependency so isolated consumer tests
cannot borrow workspace feature unification. Preserve source archive untouched.
Verify old-source v2 golden records and reverse old-reader compatibility in a
disposable source copy; retain app command/fact evolution example without changing
old request identities/encoding. Strengthen existing corruption owners rather
than add duplicated parser tests. A real public consumer must write a multi-fact
record and no-op, checkpoint state/physical position together, reload and resume
with neither lost nor repeated facts. Falsifiers: forging unvalidated public
records through Serde, copied parser/schema, changed bytes/name/schema, missing
corruption detection/implicit repair, fact-level positions, skipped/no-op progress,
new Clone/Serialize read requirements or weakened prior laws. Native persistence/
migration and final package consumer qualification remain CP6/CP7/CP9 owners.
CP4 test-budget refinement before stream/evolution edits: increase test ceiling
from500 to650 net lines, keep <=8 code/fixture owners, <=250 product lines and
two new root types. Public projection uses260 lines; the strengthened canonical
unit laws add164. Remaining acceptance needs actual processor reads/retries over
corrupt stored history (showing no repair writes), plus an application retaining
both old command and fact representations through a new variant. Neither a
stateless parser law nor a fact-only upcast proves these requirements. Put both
in existing adaptation.rs and its serial handler-count owner, not another runner,
store or duplicated parser. No production behavior/config/package change.
CP4 falsifiers before running: use cargo-mutants to replace the public Acceptance
DomainEvent name with an empty string, selecting the real public reader test.
Its first failure must be exact event-name validation after real committed writes.
The generator's empty-facts candidate uses Vec::leak inside a const getter and is
compiler-unviable, so it cannot qualify fact consumption. Separately replace only
the new getter's `self.0.facts.as_slice()` with `&[]` during a bounded one-off test
command, retaining an exact source backup and restoring it immediately via a shell
trap. This const-valid control must fail the real projection's exact persisted
model, not compilation/setup. No mutant is retained; no gate/filter is weakened.
CP3 parent integration before merge: use a separate worktree of existing signed
feat/command-races and merge exact26c5a334 CP2 parent without committing. Preserve
all tests/records from both sides, including parent readiness capacity assertion,
bounded shutdown helper and diagnostic-only mutation policy. Product source must
remain byte-identical to CP2; forced races remain unchanged. No new code owner,
API/dependency/workload/gate change. Reconcile required records explicitly; normal
full gate and signed protected delivery still apply after ancestry is resolved.
CP4 adversarial review found a masked stream witness before finalization: duplicate
second ID also exceeded the selected aggregate's one-fact cumulative limit, so
that stream could stay red if duplicate detection disappeared. Make that duplicate
an otherwise valid no-op (zero facts/events, Ignore command); keep the separate
unique-ID cumulative overflow stream unchanged. Same owners/budget, strictly
stronger defect isolation; rerun the actual adaptation owner after the edit.
CP5 lifecycle characterization before edits: separate feat/command-lifecycle
worktree from signed26c5a334 (same host/engine as CP3/CP4). Start with <=200 test
lines in existing recovery.rs plus records; no product change. Reproduce a
ready-permit release waking a caller callback that reenters stop while Admission
is locked. Isolate this synchronous deadlock witness in the same test executable
as an explicitly selected child, using std::process, a finite parent deadline,
kill+wait and captured stage markers; no runner/framework/helper binary. Also
force stop synchronously from the queue-capacity waker during receiver dequeue;
queued work must reject without running after stop. Existing FaultStore/Gate and
actual InMemoryStore supply the append boundary. Falsifiers: setup timeout instead
of selected boundary, missing callback entry, no forced overlap, additional rows,
unobserved task/child completion. Keep red evidence before selecting a locking fix.
CP4 parent integration before edits: preserve a complete binary patch/index and
named stash of this goal's current reader work, leaving all older/user stashes
untouched. Fast-forward feat/command-reader from0dc087f8 to signed7534d590 (current
CP3 source passes all12 checks/1629 tests and normal hooks, PR415 CI pending), then
apply the exact saved stash. Retain all stronger parent tests and every CP4 source,
fixture/evidence/record. Resolve overlapping histories without discarding unique
facts; current PROGRESS/PRD/backlog must reflect actual merged versus local status.
No new product/format/API/package change beyond the registered CP4 slice. Final
combined full gate still required; keep the stash and receipt until delivery.

CP4 repaired-parent integration, before mutation: preserve the complete staged
reader work and all older/user stashes. Save exact binary diff and status, create
one named retained stash, fast-forward7534d590 to corrected CP3c3a16f3c, then apply
that exact stash without dropping it. Union append-only records, retain newer
current status, and prove all CP4 code/fixtures equal the saved work. Inherit only
the two repaired race fixture owners. Budget: existing six record owners plus
inherited faults/races; no additional CP4 product/API/format/dependency changes.
Requalify all12 host checks on combined source; previous1633 pass predates this
known parent defect and cannot qualify final delivery. Abort on lost source,
removed stronger assertions, changed workloads or unrelated work mutation.

## CP5 first lifecycle repair — pre-edit budget

Two actual red laws are retained in cp5-lifecycle-before-strengthened.log:
stop from a queue-capacity wake during dequeue still accepts the queued command;
canceling an assigned-but-unpolled readiness future wakes a callback reentering
stop and deadlocks while Admission is held. Initial close-only probe was weak:
ReadyToSend.close retains its permit, so that passing child never ran its callback.
Use the strengthened actual PollSender boundary, not that false-positive claim.

Budget: host.rs and existing recovery.rs plus required records; <=50 net product
lines, <=200 test lines, no new public type/dependency/runtime/tool/schema. Poll
readiness outside Admission; it reserves capacity only, while call/send still
serialize actual admission with stop. Release stopped reservations outside the
gate. Clone caller IDs before locking, retain failed send values until the guard
is released, and reject dequeued work if stop occurred before startup. Reuse the
existing watch lifecycle value; no second flag/counter/linearization authority.
Startup is the final lifecycle check immediately before execution, allowing only
work that started before stop to settle. No caller Clone/Drop/Waker callback may
run under the admission lock. Keep active work sequential and original acceptance,
uncertainty and exact retry unchanged. Falsifiers: either original red stays red,
successful admission after stop, callback not exercised, stranded response/join,
extra rows, removed cause or changed ordering. Run targeted laws then all owner
debug/release/lint and full source gates; CP5 remains incomplete until task-failure
custody, all resource/cancellation/runtime cases and required telemetry qualify.

## CP5 completion custody — characterization budget before edits

Native JoinHandle<Shutdown> returns Err on observed abort/panic, losing the
report's original request even though the engine held uncertainty. Add one
existing recovery-owner law: hold an actual delayed append, abort and observe
completion, observe the caller's Uncertain result, then release/join the delayed
server operation before asserting report custody. Original source must fail at
the missing report, not setup or unobserved server work. Retry after resumption
must return the single original acceptance. Budget this characterization <=60
test lines in recovery.rs; no product change before the red evidence.
Then compare shared active pending versus a final per-worker Drop handoff. Prefer
moving the one owned original on Engine Drop, read only after native join, if it
preserves every cause/report and needs fewer hot-path clones/locks. No detached
monitor or new framework/runtime. Implementation budget: engine/host/lib plus
owning recovery/common finish consumers (<=8 owners), <=200 net product and
<=250 test lines for this slice, one justified new public completion owner
(cumulative target11 roots instead of10). Concrete native Err excludes Shutdown;
existing AbortOnDropHandle has the same result and cannot fix custody alone.
Retain native JoinError/panic payload, exact request and borrowed report on failed
resume. No None report may become rollback proof. Measure runtime/allocations in
CP6 before claiming an improvement. More types/state require another counterexample.
CP5 custody controls before execution: use the repository Nix cargo-mutants binary
explicitly (cargo's external-plugin lookup selected a stale host installation and
failed before metadata/tests; do not repair/install host tooling). Select generated
Engine::drop removal and Shutdown::cause returning None. Run the four real custody
owners, including the new exact original/abort law. The baseline must pass; each
control must fail at lost original or native cause, not compilation/setup/timeout.
Retain raw counts/first assertions and three reviews for every actual invocation.
No permanent production mutation, test filter reduction or additional framework.

## CP5 resource coverage — before edits

Threat: request bytes and accepted-record bytes are independent; a valid4KiB
request plus receipt/facts can exceed the record bound. A definite encode failure
must append nothing and leave that ID usable, unlike any uncertain append. The
canonical request-byte owner also lacks UTF8/JSON-escape witnesses at4095/4096/
4097. Extend that owner; add one public processor sequence using existing actual
InMemoryStore and a small text-command aggregate, complete raw rows/receipt/state/
handler-count assertions, exact retry and observed normal shutdown. No product
change is justified without an actual counterexample. Reuse existing codecs.
Budget: existing unit/model.rs and recovery.rs plus README/records, <=150 net
additional test lines, no public/API/format/dependency/runtime change. Retain the
original ASCII4096/4097/write-zero assertions and every existing workload/gate.
Falsifiers: new writer computes chars rather than actual JSON bytes; record
failure is uncertain or writes data; failed ID is spent; same ID success/retry
executes twice; handler runs for an oversized input. New tests remain unexecuted
until the independent CP4 full gate completes; no concurrent local compilation.
Timer-disabled runtime remains a separate witness: Tokio1.53.1 timeout source
explicitly panics without enable_time; try_current alone cannot prove support.
Use observed native task cause and a bounded child if needed, not an unbounded
join or detached blocking test task. Select/doc behavior after actual execution.
Resource budget refinement before edits: within the same150 net test-line ceiling,
extend the canonical largest-queue witness to MAX_PERMITS-1 and MAX_PERMITS (the
existing +1 and zero-duration rejection stay intact), and verify actual acceptance
and observed shutdown at1ns/Duration::MAX-1ns/Duration::MAX. Locked Tokio timeout
polls an immediately-ready future first and uses checked deadline addition; real
InMemoryStore should settle these immediate operations. Any failure stays visible;
do not replace the tiny bound, relax an assertion or change processor policy to
obtain a pass. Run only after CP4 compilation/gate is no longer active.

## CP5 observability — research before edits

Required features remain tracing0.1.44 and metrics0.24.6; no exporter/global
collector/runtime/framework in normal dependencies. Existing CP0 metadata owns
license/maintenance/compatibility choice. Before product instrumentation, write
one canonical recording-subscriber/recorder workload over actual public processor
calls, expected counts/labels/stages, independent caller contexts, cancellation,
uncertainty/reconciliation and observed shutdown/panic. Compare maintained
metrics-util DebuggingRecorder with a test-only Recorder implementation; prefer
supported facilities when they reduce total code/dependencies. Applications own
OpenTelemetry SDK/export. Preserve all four feature combinations/no collector.

Falsifiers include leaked caller span, wrong dispatcher after queue transfer,
duplicate counted as new acceptance, leaked/negative gauges, missing terminal
cause/original, payload/ID/error-string labels, changed receipt/admission, or any
collector/user callback while Admission/handoff locks are held. Capture tracing
Span AND Dispatch at admission if needed: async Instrument enters a span, while
WithSubscriber supplies its dispatcher per poll. Verify pinned source before
selection; do not retain entered guards across await.

Gauge causality needs proof. Queued capacity is bounded by native permits;
callbacks after enqueue may reorder decrement before increment. Compare supported
Tokio try_reserve with pre-enqueue counted guards to preserve order and existing
stop/enqueue linearization; readiness reservations and queued/in-flight calls
must remain distinct. No parallel protocol authority/boolean mode or observer
framework. Each lifecycle/permit owns its balanced guard; clone/drop/abort paths
must reject the original counterexamples. Metrics are observations, not a ledger.
Pending request stays authoritative in Engine; count host transitions from its
Option values, without a second unresolved flag. Exact native Worker completion
owns observed failure accounting. Shutdown timing must name its actual start/end;
do not substitute host lifetime or fabricated zero samples for unknown timing.

Initial implementation budget (amend before further scope): root manifest/lock,
command manifest and engine/host/model/lib plus one private observe owner (eight
product/config owners), <=300 net production lines and no public root addition;
canonical telemetry integration owner <=300 lines plus required records/docs.
Reconsider if the complete contract cannot fit; never omit a required instrument
or weaken assertions to fit a line budget. First qualify dependency/testing
facilities and write explicit workload/count oracle. Performance/allocations and
no-op/enabled-uncollected/collected overhead remain CP6, no proxy-only claims.

CP5 parent integration before mutation: preserve every staged/unstaged/untracked
lifecycle/custody/resource record and all older/user stashes. Save full binary
patch/status/source hashes, create a named retained stash, fast-forward26c5a334
to signed CP4 integration786dce713004cf90e8e3c943c0f7ee6eb3d38425, then apply
that exact stash. Union unique append-only records and preserve current status.
Retain public Acceptance/AcceptanceCodec and new Worker, both independent race
gates/shared backend, stronger capacity/teardown assertions and all golden/
projection/evolution/corruption laws. No unique source/evidence may be lost.
Budget: existing source/test owners and incoming parent files plus required
records, no additional product/API/dependency/format change. Any required typed
Worker test-helper adjustment belongs with its existing owner, <=20lines. Run
combined owner/lint/release then all12 source gates before a focused lifecycle PR;
required tracing/metrics stays a separate CP5 slice, not removed from acceptance.

CP5 observability workspace preparation before mutation: while core fullgate1166
runs, use standard Git worktree feat/command-observe at the same786dce7130 parent,
apply an exact saved binary copy of current core work, and preserve core/stashes.
Only this separate candidate gains tracing/metric edits; source used by the core
gate stays frozen. Save source/status/hash receipts. After core signed commit,
merge that parent into the candidate while preserving every novel observation
change. No compilation overlaps core gate. Source/metadata inspection is allowed.
This is ordinary isolated source work, not a new project tool or workflow.
Tracing slice first: optional existing tracing/std dependency and private context,
caller Span+Dispatch across mailbox, typed stage/outcome events and no payloads.
Before instrumentation, add the actual recording-subscriber witness and preserve
its red. Budget<=200net trace production lines, <=300test/helper lines; may reuse
neutral archived fixtures through existing test owners, no public root or runtime.
Metrics remains required as its own subsequent slice; no acceptance removed.

CP5 core protected-parent integration before mutation: origin/main8fc562836ce5
has exactly the24680399552aca8b52f383bcc3c79862669401b3 tree of already-imported
CP4786dce7130, which is an ancestor of signeda8f66b7a. There are no unique incoming
source or record facts. Preserve current clean core and registered three-review
full gate; merge main with --no-commit, resolve six history/export conflicts to
our tested complete owners, and require the entire staged tree to equal a8f66b7a
before adding only this receipt. Normal signed git commit must execute the
established all12-check hook. No compiler/test/runtime change, no new API or
workload, no telemetry mixed into this focused PR. New head requires fresh CI.
Tracing characterization fixture before edits: two callers with distinct local
subscribers submit through one real processor while a delayed append holds the
worker and the second call waits in its bounded queue. Drop the second response
future; it must still commit once. A subsequent observed read/exact retry proves
state/order and no new acceptance on replay. Capture library span parents,
stages, typed outcomes and fields independently in both subscribers; worker span
must not become request parent. Inspect complete library field/key sets, no
payload/ID/error-string fields. All tasks/server work observed before assertions.
Reuse the neutral Counter/Operation/Sum fixture by extracting its existing test
owner into support/counter.rs; history/property workloads and record bytes stay
unchanged. This adds two fixture/test owners under the existing300test-line
budget; no product export or new runtime/framework. First feature-on run must
fail for missing library observations after SUT behavior assertions, not setup.
Tracing production budget refined BEFORE instrumentation: <=250 net lines (the
PRD per-slice ceiling), five private Rust owners plus command manifest/lock.
The actual characterization requires dispatcher propagation as well as Span
ownership, a terminal outcome on interrupted call drop, worker lifetime separate
from caller parents, and typed load/write/reconcile/decision stages. Those are
proven distinctions, not a generic observer API. Preserve the300 test/helper
budget and public root count11; no additional runtime, exporter or global setup.

CP5 tracing parent integration before mutation: preserve the complete tracked
binary diff, staged diff, status, all product/test hashes, untracked trace owner
and existing stash list in cp5-observe-before-core-parent*. Create one named
retained stash including untracked source, fast-forward786dce7130 to signed core
a8f66b7af8c8e82a04d9de9f42b43e58d5660025, then apply that exact stash. Union
append-only records; retain the incoming current inventory/PRD/gate evidence
while preserving all novel tracing records and code. No added product behavior
in this integration; prove every saved product/test hash identical afterwards.
Do not remove the stash or touch older/user stashes. CP4 merge receipts belong
in this next branch, leaving its reviewed/published head unchanged.

Tracing cleanup qualification before edits: extend the test/helper ceiling from
300 to450 lines, still below the PRD500-line per-slice limit. The real prior
cross-subscriber panic requires separate observed abort/unwind/unpolled cleanup
witnesses, not just successful requests. Capture span close callbacks and finite
field VALUES as well as keys. Abort while a second subscriber's load is blocked,
retain the native cancellation, observe all queued replies and span cleanup.
Force a domain-handler panic under another subscriber and retain its exact73
payload. No dropped native cause may be hidden by another registry panic.
All selected stages/parents/outcomes and exact rows remain asserted; no global
subscriber, arbitrary sleep, extra production API or gate waiver. Product stays
within250net lines until a further justified pre-edit amendment.

Tracing cleanup production control before mutation: save the exact private
trace.rs and source hash; temporarily omit ONLY Scope's dispatcher-bound Drop,
leaving its own span reference, two independent recording subscribers, bounded
queue, server completion and all acceptance assertions unchanged. Run the
original caller-context law once; require actual intended native registry panic/
closed worker failure, not compilation or fixture setup. Restore the exact
source regardless of outcome and verify its hash. Keep raw failed log and
three distinct reviews. This control adds no retained product/test owner or API.

Next telemetry slice must first characterize missing admission/queue/shutdown
trace stages and stage outcomes; existing load includes replay, append includes
decision/encode/persist/fold, not just adapter IO. Invalid requests currently
validate before tracing and must be observed without adding another admission
path. Metrics remain required; this partial tracing candidate is not complete.

Metrics design constraints for the next experiment: use only the qualified
optional metrics facade and supported dev-only registry storage after actual
lock/dependency cost verification. A recording workload must force active,
queued and reserved states and inspect NONRESETTING counters/gauges before and
after cancellation/stop/abort. Native permits remain the authority; prepare
queue observations outside Admission while owning capacity, before enqueue,
so the worker cannot decrement a gauge that has not been incremented. Repeated
readiness and clones must not duplicate reservation meters; a retained native
permit stays counted until used/released even after worker exit. Recording
callbacks and dropped send values remain outside the admission mutex.

Engine.pending is the only unresolved-write authority. Derive gauge transitions
from its existing Option changes and cleanup, not a second protocol flag. Count
worker-reported outcomes and dropped reply sends as distinct observations;
do not create a new response future wrapper or pretend a canceled caller proves
rollback. A late commit observed while reconciling the retained exact pending
request is a newly observed acceptance once, while ordinary replay must not
emit acceptance. Advancing competitors without the original receipt are not
acceptances. Existing earlier receipts must not clear unrelated pending writes.
Test all these distinctions before instrumenting their owner.

Shutdown duration needs the actual first stop time and observed completion,
not a fabricated zero or just a successful join timestamp. If necessary extend
the existing Lifecycle::Stopping variant with its first timestamp; do not add
a parallel boolean/clock authority. Abort/panic have typed native completion
causes; unknown timing is omitted with documented scope. Trace cleanup remains
bound to its captured dispatcher on successful, unpolled, aborted and panic paths.
No entered/default-dispatch guard may cross an await.

CP5 trace-stage extension before edits: the saved237net-line tracing candidate
qualifies context and terminal cleanup but lacks required admission/queue/shutdown
and stage outcomes. Use that exact cp5-trace source patch as this experiment's
rollback boundary. First extend the owning subscriber tests and preserve their
red for missing required spans/outcomes. Extension budget<=150net production
lines in trace/host/engine/model and<=120net test/helper lines; no new dependency,
public root, runtime or framework. Combined tracing ceiling becomes387net product
and569net tests, with whole-crate1750physical product-line checkpoint. This
amends the underestimated initial250/450 before edits: the real cross-dispatch
normal/abort/panic defect requires retained context/drop machinery, while the
required operational stages still need implementation. Prefer supported tracing
scope/instrumentation and existing Inbox Drop over custom Future or monitor.

Admission spans end after actual native transfer/result outside Admission mutex;
queue spans cover the prepared handoff through dequeue, with failed transfers
explicitly classified rather than called admitted waits. Metrics later reserve
native capacity before counting queue admission. Invalid requests get a definite
request-outcome trace without reordering validation before ID clone or changing
its typed failure. Existing shared request identity/data bytes stay untouched.
Inbox owns worker trace; run explicitly marks normal completion, and Inbox Drop
records actual shutdown cleanup on normal/abort/panic paths under the captured
worker scope. The application still owns task observation; shutdown-span duration
must not be claimed as stop-to-join duration. No arbitrary successful-zero timing.

Invalid-request trace falsifier before adding its law: use the existing Counter
with an application String identity, a4097-byte identity that must fail exact
typed Bytes/WriteZero validation through BOTH submit and Tower, then a valid
small identity accepted once. The latter also proves invalid Tower calls release
their readiness permit. Assert actual decoded record/request/receipt/fact and
all request/admission outcome fields without exposing IDs/payloads. Temporarily
omit only invalid_request's trace body, require the owning law to fail for the
missing two request observations after successful SUT checks, restore exact
source/hash. Test/helper extension ceiling amended BEFORE edits from120to200
net lines (combined trace649), still<=500for this focused extension; no extra
production or dependency budget. Keep the earlier stage red and all other laws.

Stage minimization before edits: Call.queue stores only trace lifetime and must
not remain an Option flag when tracing is disabled. Gate that field and its
four accesses with the same tracing feature; retain the existing default const
constructor, all real native admission/worker state and all outcome semantics.
No new behavior/budget/API/fixture. Feature-off contract/performance cannot be
inferred from ZST Trace alone; measure actual disabled overhead in CP6.

Subscriber-oracle minimization before edits: span IDs identify graph nodes; their
numeric sorting must not supply creation order under task migration/reuse. Keep
the existing ID lookup table but record callback creation order once in Capture,
then assert ordered request operations from that fact. Keep all exact parent,
stage/outcome/count/closure checks. This corrects the observation owner rather
than relaxing order or sorting by expected operation. Same test budget/owner,
no production API/runtime. Simplify admission's cfg body to return its phase
directly while creating queue observation before native transfer; no linearization
or typed result changes.

Trace delivery preparation before mutation: save all tracked/index binary patches,
status, source hashes and goal untracked evidence; retain one named stash, then
fast-forward a8f66b7a to signed1539509a (upstream diff ONLY10HYPOTHESIS lines).
Apply exact stash, union that one append conflict, prove all code/manifests/test
hashes unchanged and retain all prior stashes. Source gate freezes this trace
branch. Preserve current candidate source/raw logs in cp5-stages evidence before
cleaning only this goal-owned Cargo target for full-gate headroom. No other
project/user cache cleanup, source/API/workload change or gate bypass. Metrics
work can proceed in a separate ordinary worktree while compilation stays serial.

Trace PR base integration before mutation: PR417 has merged972d2a69 with tree
exactly equal to signed1539509a (29d5408e), independently verified after fetch.
The unpublished observe branch has no trace commit yet. After gate91717 is
terminal, preserve all binary/index patches, status, ten source hashes, every
untracked evidence file and the existing stash list. Retain a named full stash;
rebase the zero-commit trace branch from HEAD1539509a onto origin/main972d2a69,
then apply that exact retained stash. No published branch rewrite or archive
mutation; original1539509a remains on the clean lifecycle branch. Verify all ten
source hashes and all evidence/status, union only append-only record conflicts
if any. Full Nix source must remain byte-identical to the qualified candidate.
This updates the focused PR's ancestry without changing product behavior or
inventing a parent-only commit. Normal signed hook and protected CI still apply.
