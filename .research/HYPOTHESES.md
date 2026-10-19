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
