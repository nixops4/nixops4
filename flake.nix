{
  description = "A flake with pre-commit hooks";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";
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

          pre-commit.settings.hooks.nixpkgs-fmt.enable = true;
          pre-commit.settings.hooks.rustfmt.enable = true;
          # Override to pass `--all`
          pre-commit.settings.hooks.rustfmt.entry = lib.mkForce "${pkgs.rustfmt}/bin/cargo-fmt fmt --all --manifest-path ./rust/Cargo.toml -- --color always";

          devShells.default = pkgs.mkShell {
            inputsFrom = [ config.nci.outputs.nixops4.devShell ];
            nativeBuildInputs = [
              pkgs.rust-analyzer
              pkgs.nixpkgs-fmt
              pkgs.rustfmt
            ];
            shellHook = ''
              ${config.pre-commit.installationScript}
              echo 1>&2 "Welcome to the development shell!"
            '';
          };
        };
        flake = { };
      });
}
