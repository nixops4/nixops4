{
  description = "A flake with pre-commit hooks";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";
    nix.url = "github:tweag/nix/nix-c-bindings";
    nix.inputs.nixpkgs.follows = "nixpkgs";
    nix-cargo-integration.url = "github:yusdacra/nix-cargo-integration";
    nix-cargo-integration.inputs.nixpkgs.follows = "nixpkgs";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # https://github.com/cachix/pre-commit-hooks.nix/pull/410
    pre-commit-hooks-nix.url = "github:hercules-ci/pre-commit-hooks.nix/rustfmt-all";
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
          # Override to pass `--all`
          pre-commit.settings.settings.rust.cargoManifestPath = "./rust/Cargo.toml";

          devShells.default = pkgs.mkShell {
            name = "nixops4-devshell";
            strictDeps = true;
            inputsFrom = [ config.nci.outputs.nixops4-project.devShell ];
            inherit (config.nci.outputs.nixops4-project.devShell.env)
              LIBCLANG_PATH
              BINDGEN_EXTRA_CLANG_ARGS
              ;
            buildInputs = [
              config.packages.nix
            ];
            # Workaround: the gcc in the devshell doesn't find libc headers
            RUST_NIX_C_RAW_EXTRA_CFLAGS = "-I${pkgs.stdenv.cc.libc.dev}/include";
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
