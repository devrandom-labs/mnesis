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
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        unfilteredSrc = ./.;

        src = lib.fileset.toSource {
          root = unfilteredSrc;
          fileset = lib.fileset.unions [
            (craneLib.fileset.commonCargoSources unfilteredSrc)
            (lib.fileset.fileFilter (f: f.hasExt "snap") unfilteredSrc)
          ];
        };

        commonArgs = {
          inherit src;
          strictDeps = true;
          buildInputs = with pkgs; [ openssl ];
          nativeBuildInputs = with pkgs; [ cmake pkg-config ];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

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
            # Exclude trybuild tests — .stderr snapshots contain absolute paths
            # that differ between local and Nix sandbox environments
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
          # so it alone would not catch a std leak. Both build --no-default-features.
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
              cargo build -p mnesis --target wasm32-unknown-unknown --no-default-features
              cargo build -p mnesis-nostd-smoketest --target wasm32-unknown-unknown --no-default-features --features derive
              cargo build -p mnesis-wake-nostd --target wasm32-unknown-unknown
            '';
          });

          mnesis-nostd = craneLib.mkCargoDerivation (commonArgs // {
            inherit cargoArtifacts;
            pname = "mnesis-nostd";
            buildPhaseCargoCommand = ''
              cargo build -p mnesis --target thumbv7em-none-eabihf --no-default-features
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
              cargo build -p mnesis-store --target thumbv7em-none-eabihf --no-default-features --features subscription,export,import,snapshot,projection
            '';
          });
        };

        packages = {
        } // lib.optionalAttrs isLinux {
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
            tree
            cloc
            cargo-edit
            gh
          ];
        };
      });
}
