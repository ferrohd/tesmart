{
  description = "Rust project — pinned toolchain via rust-overlay";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

    flake-parts.url = "github:hercules-ci/flake-parts";
    systems.url = "github:nix-systems/default-linux";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = import inputs.systems;

      perSystem = { system, ... }:
        let
          pkgs = import inputs.nixpkgs {
            inherit system;
            overlays = [ (import inputs.rust-overlay) ];
          };

          # rust-toolchain.toml is the single source of truth for the channel, so the editor,
          # every shell and CI cannot drift apart. It deliberately lists no components: the
          # rustup `default` profile already carries rustc, cargo, rustfmt and clippy, and
          # anything beyond that belongs to one shell rather than all of them.
          #
          # The extra targets are the platforms we compile for but never run on. thumbv7em is
          # repeated from the toolchain file because `targets` replaces rather than extends.
          crossTargets = [
            "thumbv7em-none-eabi"
            "x86_64-pc-windows-msvc"
            "aarch64-apple-darwin"
            "aarch64-unknown-linux-gnu"
          ];
          toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

          # Read from the manifest, which is the one place the MSRV is declared. Deriving it
          # here is what makes `devShells.msrv` an actual 1.88 toolchain rather than an
          # environment variable aimed at a different one, which is how the old CI job came to
          # be silently checking stable.
          msrv = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.rust-version;

          # What `just` needs for the check recipes, and so what CI needs. Kept separate from
          # devTools because CI enters the lean shell 16 times per run: rust-analyzer alone is a
          # 141 MiB closure and mold another 109 MiB, neither of which any check uses.
          checkTools = with pkgs; [
            just
            cargo-nextest
            cargo-hack
            cargo-deny
            cargo-dist
            actionlint
            taplo
            typos
            yq-go
            git
          ];

          # Only useful to a human: watching, reviewing snapshots, previewing a release.
          devTools = with pkgs; [
            cargo-insta
            bacon
            cargo-outdated
            release-plz
          ];
        in
        {
          devShells = {
            # Development, and a superset of `ci` so that `just all` works straight from direnv.
            default = pkgs.mkShell {
              packages = [
                # rust-src is what lets rust-analyzer resolve `std`. An extension here rather
                # than in rust-toolchain.toml so that CI does not download it.
                (toolchain.override {
                  targets = crossTargets;
                  extensions = [ "rust-src" ];
                })
              ]
              ++ (with pkgs; [
                # Pinned to this toolchain, so it wins over any rust-analyzer further down PATH.
                rust-analyzer
                mold
              ])
              ++ checkTools
              ++ devTools;

              # Set here and deliberately not in `ci`. RUSTFLAGS is a single string, so
              # occupying it means any caller that wants to add a flag has to remember to append
              # rather than assign, and silently loses the linker choice if it forgets.
              RUSTFLAGS = "-C link-arg=-fuse-ld=mold";
            };

            # What CI enters. Same channel and targets as `default`, without the editor tooling
            # or anything only a human would run.
            ci = pkgs.mkShell {
              packages = [ (toolchain.override { targets = crossTargets; }) ] ++ checkTools;

              # `nix develop` keeps any variable the shell does not itself set, so entering this
              # one from a direnv-activated `default` shell drags RUSTFLAGS in with it. CI never
              # has it set, and silently building with different flags locally is precisely what
              # running the same recipes in both places is meant to rule out. Worse, mold also
              # leaks in via PATH, so without this the failure only appears on a machine that
              # has the flag but not the linker.
              shellHook = "unset RUSTFLAGS";
            };

            # One recipe's worth: `cargo check` against the oldest compiler we claim to support.
            # `minimal` is enough, since checking needs neither clippy nor rustfmt. `just` comes
            # along because the recipe is run through it.
            msrv = pkgs.mkShell {
              packages = [ pkgs.rust-bin.stable."${msrv}.0".minimal pkgs.just ];
              shellHook = "unset RUSTFLAGS";
            };
          };

          formatter = pkgs.nixpkgs-fmt;
        };
    };
}
