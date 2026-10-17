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
          # both shells and CI cannot drift apart. It deliberately lists no components: the
          # rustup `default` profile already carries rustc, cargo, rustfmt and clippy, and
          # anything beyond that belongs to one shell rather than all of them.
          toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

          # Read from the manifest, which is the one place the MSRV is declared. Deriving it
          # here is what makes `devShells.msrv` an actual 1.88 toolchain rather than an
          # environment variable aimed at a different one, which is how the old CI job came to
          # be silently checking stable.
          msrv = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.rust-version;
        in
        {
          devShells = {
            # Development.
            default = pkgs.mkShell {
              packages = [
                # rust-src is what lets rust-analyzer resolve `std`. It is an extension here
                # rather than in rust-toolchain.toml so that CI does not download it.
                (toolchain.override { extensions = [ "rust-src" ]; })
              ] ++ (with pkgs; [
                # Pinned to this toolchain, so it wins over any rust-analyzer further down PATH.
                rust-analyzer
                mold
              ]);

              # Set here and deliberately not in `ci`. RUSTFLAGS is a single string, so
              # occupying it means any caller that wants to add a flag has to remember to append
              # rather than assign, and silently loses the linker choice if it forgets.
              RUSTFLAGS = "-C link-arg=-fuse-ld=mold";
            };

            # Everything ci/run.sh needs, and nothing a human needs. Same channel as `default`.
            ci = pkgs.mkShell {
              packages = [
                # The targets we compile for but never run on. thumbv7em is repeated from
                # rust-toolchain.toml because `targets` replaces rather than extends.
                (toolchain.override {
                  targets = [
                    "thumbv7em-none-eabi"
                    "x86_64-pc-windows-msvc"
                    "aarch64-apple-darwin"
                  ];
                })
              ] ++ (with pkgs; [
                cargo-nextest
                cargo-hack
                cargo-deny
                cargo-dist
                actionlint
                yq-go
                git
              ]);

              # `nix develop` keeps any variable the shell does not itself set, so entering this
              # one from a direnv-activated `default` shell drags RUSTFLAGS in with it. CI never
              # has it set, and silently building with different flags locally is precisely what
              # running the same ci/run.sh in both places is meant to rule out. Worse, mold also
              # leaks in via PATH, so without this the failure only appears on a machine that
              # has the flag but not the linker.
              shellHook = "unset RUSTFLAGS";
            };

            # One job's worth: `cargo check` against the oldest compiler we claim to support.
            # `minimal` is enough, since checking needs neither clippy nor rustfmt.
            msrv = pkgs.mkShell {
              packages = [ pkgs.rust-bin.stable."${msrv}.0".minimal ];
              shellHook = "unset RUSTFLAGS";
            };
          };

          formatter = pkgs.nixpkgs-fmt;
        };
    };
}
