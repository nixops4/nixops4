{
  description = "A flake with pre-commit hooks";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";
    nix.url = "github:NixOS/nix";
    nix.inputs.nixpkgs.follows = "nixpkgs";
    nix-cargo-integration.url = "github:yusdacra/nix-cargo-integration";
    nix-cargo-integration.inputs.nixpkgs.follows = "nixpkgs";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    pre-commit-hooks-nix.url = "github:cachix/pre-commit-hooks.nix";
    pre-commit-hooks-nix.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = inputs@{ self, flake-parts, ... }:
    flake-parts.lib.mkFlake
      { inherit inputs; }
      ({ lib, ... }: {
        imports = [
          inputs.pre-commit-hooks-nix.flakeModule
          inputs.nix-cargo-integration.flakeModule
          ./rust/nci.nix
        ];
        systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
        perSystem = { config, self', inputs', pkgs, ... }: {

          packages.default = config.packages.nixops4-release;
          packages.nix = inputs'.nix.packages.nix;

          pre-commit.settings.hooks.nixpkgs-fmt.enable = true;
          pre-commit.settings.hooks.rustfmt.enable = true;
          pre-commit.settings.settings.rust.cargoManifestPath = "./rust/Cargo.toml";

          # Check that we're using ///-style doc comments in Rust code.
          #
          # Unfortunately, rustfmt won't do this for us yet - at least not
          # without nightly, and it might do too much.
          pre-commit.settings.hooks.rust-doc-comments = {
            enable = true;
            files = "\\.rs$";
            entry = "${pkgs.writeScript "rust-doc-comments" ''
              #!${pkgs.runtimeShell}
              set -uxo pipefail
              grep -n -C3 --color=always -F '/**' "$@"
              r=$?
              set -e
              if [ $r -eq 0 ]; then
                echo "Please replace /**-style comments by /// style comments in Rust code."
                exit 1
              fi
            ''}";
          };

          devShells.default = pkgs.mkShell {
            name = "nixops4-devshell";
            strictDeps = true;
            inputsFrom = [ config.nci.outputs.nixops4-project.devShell ];
            inherit (config.nci.outputs.nixops4-project.devShell.env)
              LIBCLANG_PATH
              BINDGEN_EXTRA_CLANG_ARGS
              ;
            NIX_DEBUG_INFO_DIRS =
              let
                # TODO: add to Nixpkgs lib
                getDebug = pkg:
                  if pkg?debug then pkg.debug
                  else if pkg?lib then pkg.lib
                  else pkg;
              in
              "${getDebug config.packages.nix}/lib/debug";
            buildInputs = [
              config.packages.nix
            ];
            nativeBuildInputs = [
              pkgs.rust-analyzer
              pkgs.nixpkgs-fmt
              pkgs.rustfmt
              pkgs.pkg-config
              pkgs.clang-tools # clangd
              pkgs.valgrind
              pkgs.gdb
              # TODO: set up cargo-valgrind in shell and build
              #       currently both this and `cargo install cargo-valgrind`
              #       produce a binary that says ENOENT.
              # pkgs.cargo-valgrind
            ];
            shellHook = ''
              ${config.pre-commit.installationScript}
              echo 1>&2 "Welcome to the development shell!"
            '';
            # rust-analyzer needs a NIX_PATH for some reason
            NIX_PATH = "nixpkgs=${inputs.nixpkgs}";
          };
        };
        flake = {
          herculesCI.ciSystems = [ "x86_64-linux" ];
        };
      });
}
