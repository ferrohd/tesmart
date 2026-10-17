#!/usr/bin/env bash
#
# The check suite. One subcommand per CI job, so that what runs on a pull request and what a
# developer runs locally are the same commands rather than two drifting copies of them.
#
#   nix develop .#ci   --command ci/run.sh <subcommand>
#   nix develop .#msrv --command ci/run.sh msrv
#
# Everything passes --locked: without it cargo may quietly resolve a different dependency
# version than the committed lockfile, which makes the msrv job in particular meaningless,
# because a dependency that raised its own minimum would fail for an unrelated reason.

set -euo pipefail

cd "$(dirname "$0")/.."

# Echo before running. Folds into a collapsible section on GitHub and prints a readable line
# anywhere else.
run() {
    if [[ -n ${GITHUB_ACTIONS-} ]]; then
        echo "::group::$*"
    else
        printf '\n\033[1m$ %s\033[0m\n' "$*"
    fi
    "$@"
    [[ -n ${GITHUB_ACTIONS-} ]] && echo "::endgroup::" || true
}

# Workspace members, newest metadata rather than a list to keep in step by hand.
members() {
    cargo metadata --no-deps --format-version 1 --locked | yq -p json -r '.packages[].name' | sort
}

cmd_fmt() {
    run cargo fmt --all --check
}

cmd_clippy() {
    # [workspace.lints] already denies clippy::all, missing_docs and unsafe_code. `-D warnings`
    # is what escalates the `pedantic = warn` tier. Passed after `--` so it applies to this
    # workspace and not to every dependency, which is what setting RUSTFLAGS would do.
    run cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
    run cargo clippy --workspace --all-targets --no-default-features --locked -- -D warnings
}

cmd_doc() {
    # --cfg docsrs matches what docs.rs itself builds, since the facade sets
    # [package.metadata.docs.rs] all-features = true.
    RUSTDOCFLAGS="-D warnings --cfg docsrs" \
        run cargo doc --workspace --all-features --no-deps --locked
}

# Feature selection is the caller's: `test`, `test --all-features`, `test --no-default-features`.
cmd_test() {
    # nextest rather than `cargo test`, for the per-test timeout in .config/nextest.toml. Five
    # pseudo terminal tests once hung forever on macOS and `cargo test` has no way to notice.
    run cargo nextest run --workspace --locked --profile ci "$@"
    # nextest does not run doctests, so they need their own pass.
    run cargo test --doc --workspace --locked "$@"
}

cmd_features() {
    # Exhaustive and self-maintaining: adding a feature anywhere extends the sweep without
    # editing this file. The hand-written matrix this replaced covered 5 of 24 combinations.
    run cargo hack clippy --each-feature --workspace --all-targets --locked -- -D warnings
    run cargo hack clippy --feature-powerset -p tesmart --all-targets --locked -- -D warnings
}

# Everything we compile for but never execute.
cmd_targets() {
    # Both halves of the protocol claim to be sans-IO and allocation free. A bare-metal target
    # with no std at all is the only honest way to check that claim. The tesmart-device build is
    # load-bearing: the whole reason the switch side is its own crate rather than a feature of
    # tesmart-proto is that it can run as firmware.
    run cargo build -p tesmart-proto --target thumbv7em-none-eabi --locked
    run cargo build -p tesmart-proto --features serde --target thumbv7em-none-eabi --locked
    run cargo build -p tesmart-device --target thumbv7em-none-eabi --locked

    # The two platforms we ship binaries for but do not run tests on. `check` rather than
    # `build` because linking for either from here would need the MSVC import libraries or the
    # Apple SDK. It still catches the whole class of error that took Windows down once already:
    # tokio-serial gates set_exclusive behind #[cfg(unix)], and calling it unconditionally is an
    # E0599 that no amount of testing on Linux reveals.
    run cargo check --workspace --all-features --all-targets --locked \
        --target x86_64-pc-windows-msvc
    run cargo check --workspace --all-features --all-targets --locked \
        --target aarch64-apple-darwin
}

# Run me in devShells.msrv, which pins the compiler to Cargo.toml's rust-version.
cmd_msrv() {
    run rustc --version
    run cargo check --workspace --all-features --all-targets --locked
}

cmd_proptest() {
    # The decoder is the component most exposed to malformed input, so it gets a longer run than
    # the default 256 cases. Deliberately plain `cargo test`: these are meant to be slow and
    # would fight nextest's timeout.
    PROPTEST_CASES=8192 run cargo test -p tesmart-proto --all-features --test decoder --locked
    # Engine against device: the test that would catch an inverted ±1, a length byte computed
    # from the wrong end, or a retry budget that cannot survive a dropped reply.
    PROPTEST_CASES=8192 run cargo test -p tesmart-device --test roundtrip --locked
}

cmd_publish() {
    # release-plz does the publishing, in dependency order, when a release pull request merges.
    # This is the pre-flight: it catches the traps on the pull request that introduces them
    # rather than halfway through a release, when some crates are already on the registry and
    # cannot be taken back.
    #
    # Both `cargo package` and even `cargo package --list` refuse to describe a package whose
    # directory has uncommitted changes, which is the right call: the answer would not describe
    # anything reproducible. CI always runs on a clean checkout, so this only bites a developer
    # running `all` mid-change, and saying so beats cargo's error as the first explanation.
    if [[ -z ${GITHUB_ACTIONS-} && -n $(git status --porcelain) ]]; then
        echo
        echo "skipping the packaging checks: the working tree is dirty."
        echo "commit or stash, or run this on a clean checkout as CI does."
    else
        # Only tesmart-proto can be packaged for real before the first release: cargo package
        # resolves regular dependencies against the registry, so every crate above the graph root
        # fails with "no matching package" until the one below it is published. --list does the
        # manifest rewrite without resolving, which is the part worth checking.
        run cargo package -p tesmart-proto --locked

        local crate
        for crate in $(members); do
            [[ $crate == tesmart-proto ]] && continue
            run cargo package -p "$crate" --list --locked
        done
    fi

    # tesmart-sim depends on tesmart-client, so a versioned dev-dependency would have to be
    # published before the crate it depends on. Cargo drops a version-less one from the packaged
    # manifest instead, which is why deny.toml also sets allow-wildcard-paths.
    echo
    echo "checking that the cyclic dev-dependency carries no version"
    local manifest
    for manifest in crates/*/Cargo.toml; do
        if grep -Eq '^tesmart-sim *= *\{[^}]*version' "$manifest"; then
            echo "::error::$manifest declares tesmart-sim with a version; publishing will deadlock"
            exit 1
        fi
    done
}

cmd_deny() {
    # Advisories, licences, duplicate versions and source registries. Policy and the reasoning
    # behind every exception live in deny.toml.
    run cargo deny check advisories bans licenses sources
}

cmd_dist() {
    # The release workflow is generated from dist-workspace.toml. If the two drift, releases
    # quietly run the stale workflow, so the generated file is checked in and verified here.
    run dist generate --check
    run dist plan
}

cmd_workflows() {
    # Only the workflows we author. tesmart-cli-release.yml is generated by `dist generate`, and
    # actionlint's bundled shellcheck has style opinions about the shell dist emits that we
    # cannot act on: editing the file would make `dist generate --check` fail, which is the check
    # that actually matters for it.
    run actionlint .github/workflows/ci.yml .github/workflows/release-plz.yml \
        .github/workflows/audit.yml
}

# Everything except msrv, which needs its own shell.
cmd_all() {
    cmd_fmt
    cmd_clippy
    cmd_doc
    cmd_test --all-features
    cmd_test
    cmd_test --no-default-features
    cmd_features
    cmd_targets
    cmd_proptest
    cmd_publish
    cmd_deny
    cmd_dist
    cmd_workflows
}

usage() {
    cat >&2 <<'EOF'
usage: ci/run.sh <subcommand> [args]

  fmt          rustfmt, check only
  clippy       all features and no default features, warnings denied
  doc          rustdoc with warnings denied, as docs.rs builds it
  test [FEAT]  nextest plus doctests; FEAT is passed through to cargo
  features     every feature alone, and the facade's full powerset
  targets      bare-metal builds, and cross-checks for the shipped platforms
  msrv         cargo check on the oldest supported compiler (run in devShells.msrv)
  proptest     the extended property test runs
  publish      cargo package pre-flight and the dev-dependency guard
  deny         advisories, licences, duplicate versions, sources
  dist         the generated release workflow is current, and the plan resolves
  workflows    actionlint
  all          everything above except msrv
EOF
    exit 2
}

[[ $# -ge 1 ]] || usage

subcommand=$1
shift

case $subcommand in
    fmt | clippy | doc | test | features | targets | msrv | proptest | publish | deny | dist | workflows | all)
        "cmd_$subcommand" "$@"
        ;;
    *)
        echo "ci/run.sh: unknown subcommand '$subcommand'" >&2
        usage
        ;;
esac
