{
  description = "Mnesis CQRS, ES, DDD, Hexagonal Arch framework";
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    # 26.05 is the last Nixpkgs release supporting Intel macOS.
    nixpkgs-intel-darwin.url = "github:nixos/nixpkgs/nixpkgs-26.05-darwin";
    utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
    fenix = {
      url = "github:nix-community/fenix";
      inputs = {
        nixpkgs.follows = "nixpkgs";
      };
    };
    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };
  outputs =
    {
      self,
      nixpkgs,
      nixpkgs-intel-darwin,
      utils,
      crane,
      fenix,
      advisory-db,
      ...
    }:
    utils.lib.eachDefaultSystem (
      system:
      let
        pkgs =
          (if system == "x86_64-darwin" then nixpkgs-intel-darwin else nixpkgs).legacyPackages.${system};
        inherit (pkgs) lib;
        isLinux = pkgs.stdenv.hostPlatform.isLinux;
        # Instantiate against the selected package set; Fenix's flake no longer
        # exports x86_64-darwin, but its pinned stable toolchain still supports it.
        fenixToolchains = import fenix {
          inherit system pkgs;
          rust-analyzer-src = fenix.inputs.rust-analyzer-src;
        };
        # Pinned stable toolchain, read from rust-toolchain.toml so rustup
        # users and the flake share one source of truth. No nightly — the
        # crates are stable-clean (issue #204). The sha256 hashes the global
        # channel manifest (platform-independent), so one value covers every
        # system; per-component binaries are fetched per-platform from it.
        rustToolchain = fenixToolchains.fromToolchainFile {
          file = ./rust-toolchain.toml;
          sha256 = "sha256-zm3dyIY2T414ZRR3EhLOvptzG6gta4WZUcawzMUWtqI=";
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
            # SQLx embeds checked-in migrations and read-only schema validation.
            (lib.fileset.fileFilter (f: f.hasExt "sql") unfilteredSrc)
            # Compile diagnostics and input fixtures must survive source filtering.
            (lib.fileset.fileFilter (f: f.hasExt "stderr" || f.hasExt "json" || f.hasExt "c") unfilteredSrc)
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
          nativeBuildInputs = with pkgs; [
            cmake
            pkg-config
          ];
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
            (craneLib.filterCargoSources path type) || (lib.hasInfix "/tests/__fuzz__/" (toString path));
        };

        # The fuzz workspace has its OWN Cargo.lock (bolero + mnesis-store path
        # dep). Vendor it separately so the check builds offline/hermetically
        # without touching the root workspace's vendored deps.
        fuzzCargoArtifacts = craneLib.vendorCargoDeps { cargoLock = ./fuzz/Cargo.lock; };

        # mnesis-postgres's DB-backed tests are built once as a cargo-nextest
        # archive, then executed by the native Postgres test hook against a live
        # PostgreSQL — so the test needs no Rust toolchain, only the archive plus
        # `cargo-nextest`. The tests skip (pass) when DATABASE_URL is unset, so
        # they are inert under Darwin's normal checks. Linux also gates the
        # native `postgres-integration` package through the flake checks.
        postgresArchive =
          feature:
          craneLib.mkCargoDerivation (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "mnesis-postgres-tests-${feature}";
              doInstallCargoArtifacts = false;
              buildPhaseCargoCommand = ''
                mkdir -p $out
                cargo nextest archive --package mnesis-postgres \
                  ${
                    if feature == "all" then
                      "--all-features"
                    else
                      lib.optionalString (feature != "default") "--no-default-features --features ${feature}"
                  } \
                  --archive-file $out/mnesis-postgres.tar.zst
              '';
              nativeBuildInputs = (commonArgs.nativeBuildInputs or [ ]) ++ [ pkgs.cargo-nextest ];
            }
          );
        postgresTests = postgresArchive "all";
        postgresFeatures = lib.genAttrs (
          [ "default" ]
          ++ lib.remove "default" (
            builtins.attrNames (builtins.fromTOML (builtins.readFile ./adapters/postgres/Cargo.toml)).features
          )
        ) postgresArchive;
        postgresLaws = {
          default = "binary(conformance_tests) & test(check_append_then_read_round_trips)";
          export = "binary(listing_tests) & test(binary_ids_list_once_across_pages_and_reopen)";
          import = "binary(atomic_tests) & test(conformance_atomic::check_atomic_multi_stream_commits_all)";
          projection = "binary(projection_tests) & test(real_projection_reloads_the_exact_pair_and_resumes_after_its_native_position)";
          snapshot = "binary(snapshot_tests) & test(conformance_snapshot::check_snapshot_absent_then_commit_then_found)";
          tracing = "binary(diagnostics_tests) & test(actual_listener_scan_and_commit_work_emit_bounded_operational_fields)";
        };
      in
      with pkgs;
      {
        checks = {
          mnesis-clippy = craneLib.cargoClippy (
            commonArgs
            // {
              inherit cargoArtifacts;
              # Whole-workspace clippy across ALL targets (lib, tests, benches,
              # examples) under the full feature set. `--all-targets` (not the
              # old `--lib`) is load-bearing: test code is only compiled — and so
              # only linted — when targets beyond the lib are built, and
              # `--all-features` is only meaningful once those targets exist. The
              # `--lib`-only gate let a test-code compile break slip in under
              # `rkyv` (issue #262); this closes that hole.
              cargoClippyExtraArgs = "--workspace --all-features --all-targets -- --deny warnings";
            }
          );

          mnesis-doc = craneLib.cargoDoc (
            commonArgs
            // {
              inherit cargoArtifacts;
              RUSTDOCFLAGS = "-D warnings";
              cargoDocExtraArgs = "--workspace --exclude mnesis-postgres --all-features --no-deps";
            }
          );

          mnesis-fmt = craneLib.cargoFmt { inherit src; };

          mnesis-toml-fmt = craneLib.taploFmt {
            src = pkgs.lib.sources.sourceFilesBySuffices src [ ".toml" ];
            # taplo arguments can be further customized below as needed
            # taploExtraArgs = "format";
          };

          mnesis-audit = craneLib.cargoAudit {
            inherit src advisory-db;
          };
          mnesis-deny = craneLib.cargoDeny { inherit src; };
          # Run tests with cargo-nextest, fused with cargo-llvm-cov for coverage.
          # withLlvmCov collapses the previous separate tarpaulin step into one
          # instrumented test run — LLVM source-based coverage, no ptrace.
          mnesis-nextest = craneLib.cargoNextest (
            commonArgs
            // {
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
              # The Compile-fail tests job in checks.yml runs these from a plain
              # checkout using rust-toolchain.toml, including on toolchain bumps.
              cargoNextestExtraArgs = "-E 'not test(compile_fail)'";
            }
          );

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
          mnesis-wasm = craneLib.mkCargoDerivation (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "mnesis-wasm";
              buildPhaseCargoCommand = ''
                cargo build -p mnesis --target wasm32-unknown-unknown
                cargo build -p mnesis-nostd-smoketest --target wasm32-unknown-unknown --no-default-features --features derive
                cargo build -p mnesis-wake-nostd --target wasm32-unknown-unknown
              '';
            }
          );

          mnesis-nostd = craneLib.mkCargoDerivation (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "mnesis-nostd";
              buildPhaseCargoCommand = ''
                cargo build -p mnesis --target thumbv7em-none-eabihf
                cargo build -p mnesis-nostd-smoketest --target thumbv7em-none-eabihf --no-default-features --features derive
                cargo build -p mnesis-wake-nostd --target thumbv7em-none-eabihf
              '';
            }
          );

          # mnesis-store no_std gate (#301). The host build is the in-crate std-leak
          # detector (with #![no_std] active a `std::` path fails to resolve even
          # though the host ships std); thumbv7em is the STRONG dep-level gate — an
          # rlib build links no allocator, so the alloc-dependent store builds
          # bare-metal even though a *binary* would need a #[global_allocator].
          # The features variant keeps the dep-free surface (subscription/export/
          # import/snapshot/projection) no_std-clean; optional codec features
          # (json/rkyv/cbor) are deliberately NOT gated — they cannot build no_std
          # today (serde_json/rkyv/crc32c pull std).
          mnesis-store-nostd = craneLib.mkCargoDerivation (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "mnesis-store-nostd";
              buildPhaseCargoCommand = ''
                cargo build -p mnesis-store --no-default-features
                cargo build -p mnesis-store --target wasm32-unknown-unknown --no-default-features
                cargo build -p mnesis-store --target thumbv7em-none-eabihf --no-default-features
                cargo build -p mnesis-store --target thumbv7em-none-eabihf --no-default-features --features subscription,export,import,snapshot,projection,tracing
              '';
            }
          );

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
          mnesis-fuzz-replay = craneLib.mkCargoDerivation (
            commonArgs
            // {
              src = fuzzSrc;
              cargoVendorDir = fuzzCargoArtifacts;
              cargoArtifacts = null;
              pnameSuffix = "-fuzz-replay";
              buildPhaseCargoCommand = ''
                (cd fuzz && cargo test --no-fail-fast)
              '';
            }
          );
        }
        // lib.optionalAttrs stdenv.hostPlatform.isLinux {
          postgres-integration = self.packages.${system}.postgres-integration;
        };

        packages =
          let
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
            mkGate =
              pkg:
              craneLib.mkCargoDerivation (
                commonArgs
                // {
                  inherit cargoArtifacts;
                  pnameSuffix = "-mutants-${pkg}";
                  nativeBuildInputs = commonArgs.nativeBuildInputs ++ [
                    cargo-mutants
                    cargo-nextest
                  ];
                  buildPhaseCargoCommand = ''
                    set -o pipefail
                    ${mutantsSweep pkg}
                    cargo run --release -p mutants-gate -- \
                      check "$out/mutants.out" "$PWD/.mutants-baselines/${pkg}.json" \
                      | tee "$out/mutants-gate-report.txt"
                  '';
                  doInstallCargoArtifacts = false;
                  doCheck = false;
                }
              );
            # The seeder/reseeder: same sweep, but `emit-baseline` instead of the
            # strict ratchet verdict. It requires a complete, clean report and
            # rejects survivors/timeouts rather than blessing an invalid baseline. Run `nix build .#mutants-sweep-<pkg> -L`, then
            # `cp result/mutants-baseline.json .mutants-baselines/<pkg>.json`, REVIEW
            # it, and commit. See mutants-gate/README.md.
            mkSweep =
              pkg:
              craneLib.mkCargoDerivation (
                commonArgs
                // {
                  inherit cargoArtifacts;
                  pnameSuffix = "-mutants-sweep-${pkg}";
                  nativeBuildInputs = commonArgs.nativeBuildInputs ++ [
                    cargo-mutants
                    cargo-nextest
                  ];
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
                }
              );
            perCrate = builtins.listToAttrs (
              lib.concatMap (pkg: [
                {
                  name = "mutants-${pkg}";
                  value = mkGate pkg;
                }
                {
                  name = "mutants-sweep-${pkg}";
                  value = mkSweep pkg;
                }
              ]) mutantCrates
            );
          in
          perCrate
          // lib.optionalAttrs isLinux {
            # The supported Postgres test hook is Linux-only. Native fixtures
            # use the same compiled archive and a fresh Unix-only database.
            postgres-integration = pkgs.stdenv.mkDerivation {
              name = "mnesis-postgres-integration";
              inherit src;
              dontBuild = true;
              doCheck = true;
              nativeCheckInputs = with pkgs; [
                openssl
                postgresql
                postgresqlTestHook
                cargo-nextest
              ];
              PGUSER = "mnesis";
              PGDATABASE = "mnesis_test";
              # Administrative fault fixtures create their own restricted role.
              postgresqlTestUserOptions = "LOGIN SUPERUSER";
              postgresqlExtraSettings = "";
              checkPhase = ''
                runHook preCheck
                export DATABASE_URL="postgres:///$PGDATABASE?host=$PGHOST&user=$PGUSER"
                test -n "$DATABASE_URL"
                psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1 -Atc 'SELECT 1'
                set -o pipefail
                cargo-nextest nextest run --no-tests=fail --test-threads=1 \
                  --workspace-remap "$PWD" \
                  --archive-file ${postgresTests}/mnesis-postgres.tar.zst \
                  -E '${postgresLaws.import}' \
                  2>&1 | tee postgres-feature.log
                cargo-nextest nextest run --no-tests=fail --test-threads=1 \
                  --workspace-remap "$PWD" \
                  --archive-file ${postgresTests}/mnesis-postgres.tar.zst \
                  -E '${postgresLaws.snapshot}' \
                  2>&1 | tee -a postgres-feature.log
                cargo-nextest nextest run --no-tests=fail --test-threads=1 \
                  --workspace-remap "$PWD" \
                  --archive-file ${postgresTests}/mnesis-postgres.tar.zst \
                  -E '${postgresLaws.projection}' \
                  2>&1 | tee -a postgres-feature.log
                cargo-nextest nextest run --no-tests=fail --test-threads=1 \
                  --workspace-remap "$PWD" \
                  --archive-file ${postgresTests}/mnesis-postgres.tar.zst \
                  -E '${postgresLaws.export}' \
                  2>&1 | tee -a postgres-feature.log
                cargo-nextest nextest run --no-tests=fail --test-threads=1 \
                  --workspace-remap "$PWD" \
                  --archive-file ${postgresTests}/mnesis-postgres.tar.zst \
                  -E 'binary(session_tests) & test(explicit_close_and_drop_release_native_views_and_allow_reopen)' \
                  2>&1 | tee -a postgres-feature.log
                cargo-nextest nextest run --no-tests=fail --test-threads=1 \
                  --workspace-remap "$PWD" \
                  --archive-file ${postgresTests}/mnesis-postgres.tar.zst \
                  -E 'binary(security_tests) & test(verified_tls_runtime_can_open_append_read_close_and_reopen)' \
                  2>&1 | tee -a postgres-feature.log
                cargo-nextest nextest run --no-tests=fail --test-threads=1 \
                  --workspace-remap "$PWD" \
                  --archive-file ${postgresTests}/mnesis-postgres.tar.zst \
                  -E 'binary(durability_tests) & test(asynchronous_connection_settings_reject_before_runtime_open)' \
                  2>&1 | tee -a postgres-feature.log
                cargo-nextest nextest run --no-tests=fail --test-threads=1 \
                  --workspace-remap "$PWD" \
                  --archive-file ${postgresTests}/mnesis-postgres.tar.zst \
                  2>&1 | tee postgres.log
                ${lib.concatStringsSep "\n" (
                  lib.mapAttrsToList (feature: archive: ''
                    cargo-nextest nextest run --no-tests=fail --test-threads=1 \
                      --workspace-remap "$PWD" \
                      --archive-file ${archive}/mnesis-postgres.tar.zst \
                      -E '${postgresLaws.${feature}}' \
                      2>&1 | tee postgres-${feature}-required.log
                    cargo-nextest nextest run --no-tests=fail --test-threads=1 \
                      --workspace-remap "$PWD" \
                      --archive-file ${archive}/mnesis-postgres.tar.zst \
                      2>&1 | tee postgres-${feature}.log
                  '') postgresFeatures
                )}
                runHook postCheck
              '';
              installPhase = ''
                mkdir -p "$out"
                cp postgres.log "$out/run.log"
                cp postgres-feature.log "$out/feature.log"
                cp postgres-*.log "$out/"
              '';
            };
          };

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};

          shellHook = ''
            #!/usr/bin/env bash
            # Non-interactive commands need neither Git configuration nor a banner.
            if [[ $- == *i* ]]; then
              git config core.hooksPath .githooks
              REPO_NAME=$(basename "$PWD")
              PROPER_REPO_NAME=$(echo "$REPO_NAME" | awk '{print toupper(substr($0,1,1)) tolower(substr($0,2))}')
              figlet -f doom "$PROPER_REPO_NAME" | lolcat -a -d 2
              cowsay -f dragon-and-cow "Welcome to the $PROPER_REPO_NAME development environment on ${system}!" | lolcat
            fi
          '';

          packages = [
            bacon
            figlet
            lolcat
            cowsay
            tmux
            cargo-hakari
            cargo-mutants
            cargo-nextest
            cargo-hack
            jq
            openssl
            postgresql
            tree
            cloc
            cargo-edit
            gh
          ];
        };
      }
    );
}
