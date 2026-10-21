# Active CI repair — command delivery, 2026-10-21

User's CP0–CP9 persistent goal is active in the command-prd worktree. This isolated
fix/command-ci branch starts at main27de3b4a, preserving all command and unrelated
release work. Live PR411 dependency gate fails rustls-webpki0.103.15 advisory
RUSTSEC-2026-0382; precise patch0.103.16 passes current cargo-deny advisories.
Normal full gate initially ran1594 passing tests and replay2/1/1 but failed unused
nextest archive installation ENOSPC. Supported Crane output flag now disables
only that unused archive, retaining coverage/test commands/native PG archives.
First retry failed dependency extraction before tests (zero new execution credit).
Serial retry --max-jobs1 passes all12 host checks:1594PASS489.678s14slow,
four existing compiler exclusions, fresh fuzz2/1/1. Each actual run received three
reviews. Raw /tmp/mnesis-command-start-20261021/ci-fix-gate-serial.log. Only this goal's generated
CP1 target cache was removed, with source/logs preserved. No global GC or gate
weakening. Next: normal signed hook (cached output gives no new runtime credit), focused
PR/current-head CI/protected merge; then integrate with pending CP0/CP1 branches.
No publication or command acceptance is claimed by this baseline CI repair.

The following completed Postgres material is retained history.

# Postgres final delivery record — 2026-10-20

Required PG0–PG9 product acceptance and publication pass; final documentation
PR must pass normal protected checks and merge before marking the persistent
goal complete. No token budget. PG10 conditional; Labcast owns command IDs,
deduplication and stale-worker fencing. Root user research edit is preserved.

Source completion407 protected e8bbb109, release404 protected9c46ec75;
42successful checks/1scheduled skip/0threads at each exact reviewed head.
Expected/actual trees d8e66f8e... and8c1690dd... match. Two fresh whole-source
contract audits DONE. Product fingerprintdc45f0ac1d4420a4eca4cfcb17f5e2e2bd7000e3857dca8b6ebacf2a40954ae2
unchanged after audits. No new production/API/dependency/test/workflow changes.

Publisher39473891856 succeeds, all9crates0.5.3present/not-yanked. Postgres
archive64a900d42938d753ad332faf351dd2a0d5c75deb9b887798fc9ca2c791e233a4 matches
primary registry;4runtime source trees and PGmigrations equal9c46, VCS records
all9c46. Registry compile7/7PASS; native1PASS.076s/0skip on freshPG18.6/on/on/on,
nextest06f08e59-2e2b-48ab-a316-efb8dd3fd036, backend0 and waited shutdown0.
All actual invocations THREEreviewed; no guarded/compiler/cached/echo native credit.
All12normal host0.5.3checks pass, workspace1594PASS549.723s/replay2/1/1.

Current branch docs/postgres-publication /tmp/mnesis-postgres-release-053 is based
protected9c46; docs/evidence-only record budget8owners/6artifacts/1400addedlines.
Canonical exact receipt docs/prds/postgres/delivery.md and acceptance/backlog.
Normal signed hook must run, then focused PR/current-head CI/0threads/expected
merge tree/ordinary exact-head squash. Source unchanged outputs may cache, never
count cached output as another runtime. After merge, record actual final PR/tree
in root resume pointer and mark goal complete. Do not start new audit waves or
rerun long capacity/soak without a material change or actual failure.

Preserve all worktrees/stashes/raw logs and the unrelated managed changelogPR408
(six core/Fjall/inmemory changelogs only; no Postgres/store change). Pending raw
review history /tmp/mnesis-postgres-publication-notes.md is appended to research.
External clean consumers /tmp/mnesis-pg9-registry-{compile,native}-053 retain exact
source/lock/metadata; database path /tmp/mnesis-pg9-registry-native-053-dbpath.
No local gate/native workload is active before this record's normal signed hook.

Final record PR409 is open. Its initial normal signed hook passed all12checks,
workspace1594PASS596.223s/15slow/4existing compiler exclusions plus replay2/1/1;
each actual invocation THREEreviewed. JSON receipt inclusion explains rebuild,
no source/filter/gate changes. Follow-up records these results only, same filtered
source; normal cached reuse gives no new runtime credit. Complete exact-head
PR409CI/threads/tree/normal squash, then mark persistent goal complete.
