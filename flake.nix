{
  description = "Mnesis CQRS, ES, DDD, Hexagonal Arch framework";
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
    fenix = {
      url = "github:nix-community/fenix";
      inputs = { nixpkgs.follows = "nixpkgs"; };
    };
    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };
  outputs = { self, nixpkgs, utils, crane, fenix, advisory-db, ... }:
    utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        inherit (pkgs) lib;
        isLinux = pkgs.stdenv.isLinux;
        # Pinned stable toolchain, read from rust-toolchain.toml so rustup
        # users and the flake share one source of truth. No nightly — the
        # crates are stable-clean (issue #204). The sha256 hashes the global
        # channel manifest (platform-independent), so one value covers every
        # system; per-component binaries are fetched per-platform from it.
        rustToolchain = fenix.packages.${system}.fromToolchainFile {
          file = ./rust-toolchain.toml;
          sha256 = "sha256-gh/xTkxKHL4eiRXzWv8KP7vfjSk61Iq48x47BEDFgfk=";
        };
        # Function form (crane >= 0.18.0): `overrideToolchain` takes a callback
        # that builds the toolchain for a given `pkgs` instantiation, for correct
        # cross-compilation splicing. We don't nix-cross (wasm/no_std targets ride
        # the host toolchain via cargo `--target`), so the argument is ignored and
        # the `${system}`-pinned `rustToolchain` is returned as-is (issue #222).
        craneLib = (crane.mkLib pkgs).overrideToolchain (_: rustToolchain);

        unfilteredSrc = ./.;

        src = lib.fileset.toSource {
          root = unfilteredSrc;
          fileset = lib.fileset.unions [
            (craneLib.fileset.commonCargoSources unfilteredSrc)
            (lib.fileset.fileFilter (f: f.hasExt "snap") unfilteredSrc)
            # Per-crate mutation-gate ratchets (`.mutants-baselines/<crate>.json`).
            # crane's cargo-source filter strips non-.rs/.toml files, but each
            # `mutants-<crate>` verdict step reads its baseline from the sandbox.
            # `maybeMissing` keeps eval clean for crates not yet rolled out.
            (lib.fileset.maybeMissing ./.mutants-baselines)
            # cargo-mutants' exclude list (equivalent/non-terminating mutants).
            # Must reach the sandbox or those mutants reappear as survivors.
            ./.cargo/mutants.toml
          ];
        };

        commonArgs = {
          inherit src;
          strictDeps = true;
          buildInputs = with pkgs; [ openssl ];
          nativeBuildInputs = with pkgs; [ cmake pkg-config ];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # Source for the isolated `fuzz/` workspace check. crane's cargo-source
        # filter keeps only .rs/.toml/Cargo.lock, which would strip committed
        # corpus seeds under `fuzz/tests/__fuzz__/**` (they have no extension).
        # bolero's DefaultEngine replays those seeds, so they MUST reach the
        # sandbox. This filter keeps everything crane keeps PLUS any file under a
        # `__fuzz__` corpus directory.
        fuzzSrc = lib.cleanSourceWith {
          src = ./.;
          name = "mnesis-fuzz-source";
          filter =
            path: type:
            (craneLib.filterCargoSources path type)
            || (lib.hasInfix "/tests/__fuzz__/" (toString path));
        };

        # The fuzz workspace has its OWN Cargo.lock (bolero + mnesis-store path
        # dep). Vendor it separately so the check builds offline/hermetically
        # without touching the root workspace's vendored deps.
        fuzzCargoArtifacts = craneLib.vendorCargoDeps { cargoLock = ./fuzz/Cargo.lock; };

        # mnesis-postgres's DB-backed tests are built once as a cargo-nextest
        # archive, then executed *inside* the NixOS VM below against a live
        # PostgreSQL — so the VM needs no Rust toolchain, only the archive plus
        # `cargo-nextest`. The tests skip (pass) when DATABASE_URL is unset, so
        # they are inert under the normal `nix flake check`; this archive + the
        # Linux-only `postgres-integration` package are what actually run them.
        postgresTests = craneLib.mkCargoDerivation (commonArgs // {
          inherit cargoArtifacts;
          pname = "mnesis-postgres-tests";
          doInstallCargoArtifacts = false;
          buildPhaseCargoCommand = ''
            mkdir -p $out
            cargo nextest archive --package mnesis-postgres \
              --archive-file $out/mnesis-postgres.tar.zst
          '';
          nativeBuildInputs = (commonArgs.nativeBuildInputs or [ ])
            ++ [ pkgs.cargo-nextest ];
        });
      in with pkgs; {
        checks = {
          mnesis-clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            # Whole-workspace clippy across ALL targets (lib, tests, benches,
            # examples) under the full feature set. `--all-targets` (not the
            # old `--lib`) is load-bearing: test code is only compiled — and so
            # only linted — when targets beyond the lib are built, and
            # `--all-features` is only meaningful once those targets exist. The
            # `--lib`-only gate let a test-code compile break slip in under
            # `rkyv` (issue #262); this closes that hole.
            cargoClippyExtraArgs = "--workspace --all-features --all-targets -- --deny warnings";
          });

          mnesis-doc =
            craneLib.cargoDoc (commonArgs // { inherit cargoArtifacts; });

          mnesis-fmt = craneLib.cargoFmt { inherit src; };

          mnesis-toml-fmt = craneLib.taploFmt {
            src = pkgs.lib.sources.sourceFilesBySuffices src [ ".toml" ];
            # taplo arguments can be further customized below as needed
            # taploExtraArgs = "format";
          };

          mnesis-audit = craneLib.cargoAudit {
            inherit src advisory-db;
            # RUSTSEC-2026-0009: time 0.3.x DoS — transitive dep from refinery 0.8.x
            # Cannot fix until refinery upgrades to rusqlite 0.39+
            cargoAuditExtraArgs = "--ignore RUSTSEC-2026-0009";
          };
          mnesis-deny = craneLib.cargoDeny { inherit src; };
          # Run tests with cargo-nextest, fused with cargo-llvm-cov for coverage.
          # withLlvmCov collapses the previous separate tarpaulin step into one
          # instrumented test run — LLVM source-based coverage, no ptrace.
          mnesis-nextest = craneLib.cargoNextest (commonArgs // {
            inherit cargoArtifacts;
            withLlvmCov = true;
            partitions = 1;
            partitionType = "count";
            # Exclude trybuild tests. The committed .stderr snapshots are
            # themselves path-relative, but trybuild *runs* cargo inside the
            # sandbox against `/nix/var/nix/builds/.../source` with a vendored
            # `/nix/store/...-vendor-cargo-deps` registry, and the diagnostics
            # it produces there do not match snapshots blessed on a normal
            # checkout. Verified 2026-07-24: dropping this filter fails all
            # four trybuild tests in the sandbox while they pass locally.
            #
            # KNOWN GAP: nothing else runs these, so a snapshot can rot
            # unnoticed — `version_literal/zero.stderr` drifted against a
            # rustc caret-span change and sat broken for ~4 months, invisible
            # on a protected branch. Closing that needs a trybuild run outside
            # the Nix sandbox (plain CI job), not removal of this filter.
            cargoNextestExtraArgs = "-E 'not test(compile_fail)'";
          });

          # Ensure that cargo-hakari is up to date
          mnesis-hakari = craneLib.mkCargoDerivation {
            inherit src;
            pname = "mnesis-hakari";
            cargoArtifacts = null;
            doInstallCargoArtifacts = false;

            buildPhaseCargoCommand = ''
              cargo hakari generate --diff  # workspace-hack Cargo.toml is up-to-date
              cargo hakari manage-deps --dry-run  # all workspace crates depend on workspace-hack
              cargo hakari verify
            '';
            nativeBuildInputs = [ cargo-hakari ];
          };

          # no_std gates — CI is just `nix flake check`, so these ride along.
          # `mnesis-nostd` (thumbv7em-none-eabihf) is the STRONG gate: a fully
          # std-free bare-metal target. `wasm32-unknown-unknown` still ships std,
          # so it alone would not catch a std leak. Both build the kernel and
          # smoke consumer with Mnesis dependency defaults, proving no_std does
          # not require a downstream opt-out (#364).
          # Each gate also builds `mnesis-nostd-smoketest` (#304): a crate that
          # uses `#[mnesis::aggregate]` + `#[derive(DomainEvent)]`, so the macro
          # OUTPUT — not just its source — is compiled for the target. A macro
          # emitting a `std::` path fails the thumbv7em build here.
          # `mnesis-store-nostd` (#301) is the store-crate sibling gate — see its
          # own comment below for what host vs thumbv7em each catch there.
          # `mnesis-wake-nostd` (#302) also builds on both targets: the no_std
          # WakeSource bridge, proving event-listener + the wake traits are
          # core+alloc clean.
          mnesis-wasm = craneLib.mkCargoDerivation (commonArgs // {
            inherit cargoArtifacts;
            pname = "mnesis-wasm";
            buildPhaseCargoCommand = ''
              cargo build -p mnesis --target wasm32-unknown-unknown
              cargo build -p mnesis-nostd-smoketest --target wasm32-unknown-unknown --no-default-features --features derive
              cargo build -p mnesis-wake-nostd --target wasm32-unknown-unknown
            '';
          });

          mnesis-nostd = craneLib.mkCargoDerivation (commonArgs // {
            inherit cargoArtifacts;
            pname = "mnesis-nostd";
            buildPhaseCargoCommand = ''
              cargo build -p mnesis --target thumbv7em-none-eabihf
              cargo build -p mnesis-nostd-smoketest --target thumbv7em-none-eabihf --no-default-features --features derive
              cargo build -p mnesis-wake-nostd --target thumbv7em-none-eabihf
            '';
          });

          # mnesis-store no_std gate (#301). The host build is the in-crate std-leak
          # detector (with #![no_std] active a `std::` path fails to resolve even
          # though the host ships std); thumbv7em is the STRONG dep-level gate — an
          # rlib build links no allocator, so the alloc-dependent store builds
          # bare-metal even though a *binary* would need a #[global_allocator].
          # The features variant keeps the dep-free surface (subscription/export/
          # import/snapshot/projection) no_std-clean; optional codec features
          # (json/rkyv/cbor) are deliberately NOT gated — they cannot build no_std
          # today (serde_json/rkyv/crc32c pull std).
          mnesis-store-nostd = craneLib.mkCargoDerivation (commonArgs // {
            inherit cargoArtifacts;
            pname = "mnesis-store-nostd";
            buildPhaseCargoCommand = ''
              cargo build -p mnesis-store --no-default-features
              cargo build -p mnesis-store --target wasm32-unknown-unknown --no-default-features
              cargo build -p mnesis-store --target thumbv7em-none-eabihf --no-default-features
              cargo build -p mnesis-store --target thumbv7em-none-eabihf --no-default-features --features subscription,export,import,snapshot,projection,tracing
            '';
          });

          # Deterministic corpus-replay + bounded-random fuzz gate (ported from
          # the sibling `cesr` repo). Runs the bolero `check!` targets in the
          # isolated `fuzz/` workspace via plain `cargo test` on the pinned
          # STABLE toolchain — bolero's DefaultEngine needs no nightly; the
          # coverage-guided sanitizer runs (which DO need nightly) live in the
          # scheduled `deep-fuzz` GitHub workflow, quarantined off this gate.
          # The source carries the whole tree (so `mnesis-store = { path = .. }`
          # resolves) plus `fuzz/`'s committed corpus seeds; `fuzzCargoArtifacts`
          # vendors the fuzz workspace's own Cargo.lock so the build is hermetic.
          # bolero discovers corpus relative to CARGO_MANIFEST_DIR, so the test
          # runs from the fuzz workspace root where `tests/__fuzz__/**` resolves.
          mnesis-fuzz-replay = craneLib.mkCargoDerivation (commonArgs // {
            src = fuzzSrc;
            cargoVendorDir = fuzzCargoArtifacts;
            cargoArtifacts = null;
            pnameSuffix = "-fuzz-replay";
            buildPhaseCargoCommand = ''
              (cd fuzz && cargo test --no-fail-fast)
            '';
          });
        };

        packages = let
          # Mutation gate is rolled out ONE CRATE AT A TIME (a whole-workspace
          # sweep is 1542 mutants / many hours — over GitHub's 6h job cap). Each
          # crate gets its own `.#mutants-<pkg>` verdict + `.#mutants-sweep-<pkg>`
          # seeder and its own `.mutants-baselines/<pkg>.json` ratchet. The CI
          # nightly matrix (mutants.yml) gates the crates that have a committed
          # baseline; PRs run `cargo-mutants --in-diff` instead (changed lines
          # only). `mnesis-postgres` is absent by decision — its tests need a live
          # DB the sandbox lacks; mutation-grading it is tracked in #351.
          mutantCrates = [
            "mnesis"
            "mnesis-store"
            "mnesis-fjall"
            "mnesis-inmemory"
            "mnesis-wake"
            "mnesis-wake-nostd"
            "mnesis-macros"
          ];
          # One cargo-mutants sweep of a single crate. `--test-tool nextest` runs
          # the suite in PARALLEL (plain `cargo test` runs proptest serially and
          # blows past any sane per-mutant timeout on the baseline alone).
          # PROPTEST_CASES is capped so a few cases kill each mutant (the full
          # 256-case run stays the normal gate's job). NO fixed --timeout:
          # cargo-mutants auto-derives it from the MEASURED baseline, robust to
          # machine load that would clip a fixed cap into a false timeout.
          # `-- -E 'not test(compile_fail)'` forwards to nextest: the trybuild
          # compile-fail tests can't match their .stderr snapshots in the sandbox
          # (path-relative), so the normal nextest gate excludes them too — without
          # it the unmutated baseline fails and zero mutants get tested.
          mutantsSweep = pkg: ''
            PROPTEST_CASES=64 cargo mutants \
              --package ${pkg} \
              --test-tool nextest \
              --no-shuffle --colors never \
              --output "$out" \
              -- -E 'not test(compile_fail)' || true
          '';
          # The gate: sweep one crate, then `mutants-gate` OWNS the verdict — fails
          # on a survivor, timeout, interrupted run, or viability collapse against
          # `.mutants-baselines/<pkg>.json`, and writes the ratio to
          # $out/mutants-gate-report.txt. cargo-mutants' own exit is swallowed
          # (`|| true`); `set -o pipefail` makes the gate's exit fail the build.
          # Fails closed until that crate's baseline is committed.
          mkGate = pkg: craneLib.mkCargoDerivation (commonArgs // {
            inherit cargoArtifacts;
            pnameSuffix = "-mutants-${pkg}";
            nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ cargo-mutants cargo-nextest ];
            buildPhaseCargoCommand = ''
              set -o pipefail
              ${mutantsSweep pkg}
              cargo run --release -p mutants-gate -- \
                check "$out/mutants.out" "$PWD/.mutants-baselines/${pkg}.json" \
                | tee "$out/mutants-gate-report.txt"
            '';
            doInstallCargoArtifacts = false;
            doCheck = false;
          });
          # The seeder/reseeder: same sweep, but `emit-baseline` instead of the
          # strict verdict, and it ALWAYS succeeds — so it produces a fresh
          # baseline even when none exists yet (the gate fails closed and cannot
          # seed itself). Run `nix build .#mutants-sweep-<pkg> -L`, then
          # `cp result/mutants-baseline.json .mutants-baselines/<pkg>.json`, REVIEW
          # it, and commit. See mutants-gate/README.md.
          mkSweep = pkg: craneLib.mkCargoDerivation (commonArgs // {
            inherit cargoArtifacts;
            pnameSuffix = "-mutants-sweep-${pkg}";
            nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ cargo-mutants cargo-nextest ];
            buildPhaseCargoCommand = ''
              ${mutantsSweep pkg}
              cargo run --release -p mutants-gate -- \
                emit-baseline "$out/mutants.out" > "$out/mutants-baseline.json"
              # Surface survivors/timeouts in the build log so a seed run that
              # reveals a genuine test gap is visible, not silently floored.
              cp -f "$out/mutants.out/missed.txt" "$out/missed.txt" 2>/dev/null || true
              cp -f "$out/mutants.out/timeout.txt" "$out/timeout.txt" 2>/dev/null || true
            '';
            doInstallCargoArtifacts = false;
            doCheck = false;
          });
          perCrate = builtins.listToAttrs (lib.concatMap (pkg: [
            { name = "mutants-${pkg}"; value = mkGate pkg; }
            { name = "mutants-sweep-${pkg}"; value = mkSweep pkg; }
          ]) mutantCrates);
        in perCrate // lib.optionalAttrs isLinux {
          # NixOS integration tests require Linux VMs — Linux-only `packages`
          # attribute, deliberately NOT a `checks` entry, so the darwin
          # `nix flake check` dev gate never builds a test archive or boots a VM.
          # Boots a NixOS VM with services.postgresql (a `mnesis_test` DB, local
          # trust auth), then runs the pre-built nextest archive against it with
          # DATABASE_URL pointing at the VM's unix-socket Postgres — so the
          # mnesis-postgres tests that skip without DATABASE_URL actually execute.
          # Run on Linux/CI with: nix build .#postgres-integration
          postgres-integration = pkgs.testers.runNixOSTest {
            name = "mnesis-postgres-integration";
            nodes.machine = { pkgs, ... }: {
              services.postgresql = {
                enable = true;
                ensureDatabases = [ "mnesis_test" ];
                # `local all all trust` lets the test process connect over the
                # unix socket as any role without a password — the simplest auth
                # for a throwaway CI VM (no networked Postgres, no TLS).
                authentication = lib.mkForce ''
                  local all all trust
                '';
              };
              environment.systemPackages = [ pkgs.cargo-nextest pkgs.zstd ];
            };
            testScript = ''
              machine.wait_for_unit("postgresql.service")
              # The unix-socket DATABASE_URL form needs a role matching the OS
              # user running the tests. The test script runs as root, so create a
              # `root` superuser role; `mnesis_test` is owned by `postgres` and
              # granted to root so the tests can create/truncate the events table.
              machine.succeed("su postgres -c \"psql -c \\\"CREATE ROLE root LOGIN SUPERUSER;\\\"\"")
              machine.copy_from_host(
                  "${postgresTests}/mnesis-postgres.tar.zst", "/tmp/tests.tar.zst"
              )
              # The nextest archive ships the compiled test binaries but NOT the
              # source, yet nextest still needs the workspace manifest at run
              # time (it records the build-time path, which doesn't exist here).
              # Copy the toolchain-free source tree in and point --workspace-remap
              # at it; chmod so nextest can write scratch under it if needed.
              machine.copy_from_host("${src}", "/tmp/src")
              machine.succeed("chmod -R u+w /tmp/src")
              # Invoke `cargo-nextest` DIRECTLY, not `cargo nextest`: the VM has
              # no Rust toolchain (only the cargo-nextest binary + the prebuilt
              # archive), so `cargo` is not on PATH. Running from an archive is
              # self-contained and needs no cargo/rustc.
              # --test-threads=1: the DB-backed tests share one `events` table
              # and isolate via TRUNCATE in setup(), so they MUST run serially —
              # parallel tests would clobber each other's rows (and race on
              # CREATE TABLE IF NOT EXISTS).
              machine.succeed(
                  "DATABASE_URL='postgres:///mnesis_test?host=/run/postgresql' "
                  "cargo-nextest nextest run --test-threads=1 "
                  "--workspace-remap /tmp/src --archive-file /tmp/tests.tar.zst 2>&1 | tee /tmp/out"
              )
            '';
          };
        };

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};

          shellHook = ''
            #!/usr/bin/env bash
            # Set git hooks path to tracked .githooks/ directory
            git config core.hooksPath .githooks
            # Create a fancy welcome message
            REPO_NAME=$(basename "$PWD")
            PROPER_REPO_NAME=$(echo "$REPO_NAME" | awk '{print toupper(substr($0,1,1)) tolower(substr($0,2))}')
            figlet -f doom "$PROPER_REPO_NAME" | lolcat -a -d 2
            cowsay -f dragon-and-cow "Welcome to the $PROPER_REPO_NAME development environment on ${system}!" | lolcat
          '';

          packages = [
            fenix.packages.${system}.rust-analyzer
            bacon
            figlet
            lolcat
            cowsay
            tmux
            cargo-hakari
            cargo-mutants
            cargo-nextest
            tree
            cloc
            cargo-edit
            gh
          ];
        };
      });
}
