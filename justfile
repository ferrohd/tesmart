# Every check CI runs, and the handful of commands that only help a human.
#
#   just              list the recipes
#   just all          the whole check suite, as CI runs it
#   just sim          an emulated switch to point things at
#
# CI runs these same recipes, one per job, so there is nothing for the two to drift apart on:
#
#   nix develop .#ci   --command just <recipe>
#   nix develop .#msrv --command just msrv
#
# Everything passes --locked. Without it cargo may quietly resolve a different dependency
# version than the committed lockfile, which makes `msrv` in particular meaningless: a
# dependency that raised its own minimum would fail for an unrelated reason.
#
# Recipes carry a [doc] attribute because `just --list` shows only the last line of a comment
# block, which turns an explanation into a non-sequitur.

default:
    @just --list --unsorted

# ---------------------------------------------------------------- checks

[doc('rustfmt, check only')]
fmt:
    cargo fmt --all --check

# [workspace.lints] already denies clippy::all, missing_docs and unsafe_code. `-D warnings` is
# what escalates the `pedantic = warn` tier. It goes after `--` so it applies to this workspace
# and not to every dependency, which is what setting RUSTFLAGS would do.
[doc('clippy, with all features and with none')]
clippy:
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
    cargo clippy --workspace --all-targets --no-default-features --locked -- -D warnings

# --cfg docsrs matches what docs.rs itself builds, since the facade sets
# [package.metadata.docs.rs] all-features = true.
[doc('rustdoc, warnings denied, as docs.rs builds it')]
doc:
    RUSTDOCFLAGS="-D warnings --cfg docsrs" cargo doc --workspace --all-features --no-deps --locked

# nextest rather than `cargo test`, for the per-test timeout in .config/nextest.toml: five
# pseudo terminal tests once hung forever on macOS and `cargo test` has no way to notice.
# nextest does not run doctests, hence the second line.
[doc('tests and doctests; FEATURES passes through to cargo')]
test *FEATURES:
    cargo nextest run --workspace --locked --profile ci {{ FEATURES }}
    cargo test --doc --workspace --locked {{ FEATURES }}

# Exhaustive and self-maintaining: adding a feature anywhere extends the sweep without editing
# this file. The hand-written CI matrix this replaced covered 5 of the facade's 24 combinations.
[doc('every feature alone, then the facade powerset')]
features:
    cargo hack clippy --each-feature --workspace --all-targets --locked -- -D warnings
    cargo hack clippy --feature-powerset -p tesmart --all-targets --locked -- -D warnings

# The bare-metal builds are load-bearing, not decorative: both halves of the protocol claim to
# be sans-IO and allocation free, and the whole reason the switch side is its own crate rather
# than a feature of tesmart-proto is that it can run as firmware.
#
# The other three are the platforms we ship binaries for but run no tests on. `check` rather
# than `build`: linking the first two from here would need the MSVC import libraries or the
# Apple SDK, and `check` is enough to catch the class of error that took Windows down once
# already — tokio-serial gates set_exclusive behind #[cfg(unix)], and calling it unconditionally
# is an E0599 that no amount of testing on Linux reveals.
#
# aarch64-unknown-linux-gnu is here for consistency rather than because it is likely to break:
# it is the same OS as the host, there are no target_arch conditionals anywhere in the
# workspace, and dist builds and links it natively on every pull request. It is the cheapest
# possible way to keep "everything we ship is compile-checked" literally true.
[doc('bare-metal builds, and cross-checks for the shipped platforms')]
targets:
    cargo build -p tesmart-proto --target thumbv7em-none-eabi --locked
    cargo build -p tesmart-proto --features serde --target thumbv7em-none-eabi --locked
    cargo build -p tesmart-device --target thumbv7em-none-eabi --locked
    cargo check --workspace --all-features --all-targets --locked --target x86_64-pc-windows-msvc
    cargo check --workspace --all-features --all-targets --locked --target aarch64-apple-darwin
    cargo check --workspace --all-features --all-targets --locked --target aarch64-unknown-linux-gnu

# Run me in devShells.msrv, which pins the compiler to Cargo.toml's rust-version:
#   nix develop .#msrv --command just msrv
[doc('cargo check on the oldest supported compiler')]
msrv:
    rustc --version
    cargo check --workspace --all-features --all-targets --locked

# Deliberately plain `cargo test`: these are meant to be slow and would fight nextest's timeout.
# The decoder is the component most exposed to malformed input, so it gets a longer run than the
# default 256 cases. Engine against device is the test that would catch an inverted ±1, a length
# byte computed from the wrong end, or a retry budget that cannot survive a dropped reply.
[doc('the extended property test runs')]
proptest:
    PROPTEST_CASES=8192 cargo test -p tesmart-proto --all-features --test decoder --locked
    PROPTEST_CASES=8192 cargo test -p tesmart-device --test roundtrip --locked

[doc('what cargo publish would do, before release-plz does it')]
publish:
    #!/usr/bin/env bash
    set -euo pipefail
    # release-plz publishes in dependency order when a release pull request merges. This is the
    # pre-flight: it catches the traps on the pull request that introduces them rather than
    # halfway through a release, when some crates are already on the registry and cannot be
    # taken back.
    #
    # Both `cargo package` and even `cargo package --list` refuse to describe a package whose
    # directory has uncommitted changes, which is the right call: the answer would not describe
    # anything reproducible. CI always runs on a clean checkout, so this only bites a developer
    # running `all` mid-change, and saying so beats cargo's error as the first explanation.
    if [[ -z ${GITHUB_ACTIONS-} && -n $(git status --porcelain) ]]; then
        echo "skipping the packaging checks: the working tree is dirty."
        echo "commit or stash, or run this on a clean checkout as CI does."
    else
        # Only tesmart-proto can be packaged for real before the first release: cargo package
        # resolves regular dependencies against the registry, so every crate above the graph
        # root fails with "no matching package" until the one below it is published. --list does
        # the manifest rewrite without resolving, which is the part worth checking.
        cargo package -p tesmart-proto --locked
        for crate in $(cargo metadata --no-deps --format-version 1 --locked |
            yq -p json -r '.packages[].name' | sort); do
            [[ $crate == tesmart-proto ]] && continue
            cargo package -p "$crate" --list --locked
        done
    fi

    # tesmart-sim depends on tesmart-client, so a versioned dev-dependency would have to be
    # published before the crate it depends on. Cargo drops a version-less one from the packaged
    # manifest instead, which is why deny.toml also sets allow-wildcard-paths.
    echo "checking that the cyclic dev-dependency carries no version"
    for manifest in crates/*/Cargo.toml; do
        if grep -Eq '^tesmart-sim *= *\{[^}]*version' "$manifest"; then
            echo "::error::$manifest declares tesmart-sim with a version; publishing will deadlock"
            exit 1
        fi
    done

# Policy and the reasoning behind every exception live in deny.toml.
[doc('advisories, licences, duplicate versions, sources')]
deny:
    cargo deny check advisories bans licenses sources

# Config, and why its include list is explicit, are in .taplo.toml.
[doc('TOML formatting')]
toml:
    taplo fmt --check --diff

# typos honours .gitignore, so it needs no exclude list of its own.
[doc('spelling, in prose and in comments')]
typos:
    typos

# The release workflow is generated from dist-workspace.toml. If the two drift, releases quietly
# run the stale workflow, so the generated file is checked in and verified here.
[doc('the generated release workflow is current, and the plan resolves')]
dist:
    dist generate --check
    dist plan

# Only the workflows we author. tesmart-cli--release.yml is generated by `dist generate`, and
# actionlint's bundled shellcheck has style opinions about the shell dist emits that we cannot
# act on: editing that file would make `dist generate --check` fail, which is the check that
# actually matters for it.
[doc('actionlint')]
workflows:
    actionlint .github/workflows/ci.yml .github/workflows/release-plz.yml .github/workflows/audit.yml

[doc('the whole suite, minus msrv, which needs its own shell')]
all: fmt clippy doc features targets proptest publish deny toml typos dist workflows
    @just test --all-features
    @just test
    @just test --no-default-features

# ---------------------------------------------------------------- development

[doc('an emulated switch on 127.0.0.1:5000, to point anything at')]
sim MODEL="HKS1601-EB23":
    cargo run -p tesmart-cli -- sim serve --model {{ MODEL }}

[doc('the CLI against an in-process emulator: `just cli switch 4`')]
cli *ARGS:
    cargo run -q -p tesmart-cli -- -t sim -m HKS1601-EB23 {{ ARGS }}

[doc('accept the inline snapshot changes in tests/cli.rs')]
snap:
    cargo insta test --accept --workspace --all-features

[doc('rebuild and retest on every save')]
watch:
    bacon

[doc('apply what clippy, rustfmt and taplo can fix themselves')]
fix:
    cargo clippy --workspace --all-targets --all-features --fix --allow-dirty --allow-staged
    cargo fmt --all
    taplo fmt

[doc('build the docs and open them')]
docs:
    cargo doc --workspace --all-features --no-deps --open

[doc('dependencies with newer versions available')]
outdated:
    cargo outdated --workspace --root-deps-only

[doc('update the lockfile and the flake inputs')]
update:
    cargo update
    nix flake update

[doc('what release-plz would do next, without doing it')]
release-preview:
    release-plz update --dry-run
