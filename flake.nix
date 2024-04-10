{
  description = "A flake with pre-commit hooks";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nix.url = "github:NixOS/nix";
    nix.inputs.nixpkgs.follows = "nixpkgs";
    nix-cargo-integration.url = "github:yusdacra/nix-cargo-integration";
    nix-cargo-integration.inputs.nixpkgs.follows = "nixpkgs";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = inputs@{ self, flake-parts, ... }:
    flake-parts.lib.mkFlake
      { inherit inputs; }
      ({ lib, ... }: {
        imports = [
          inputs.nix-cargo-integration.flakeModule
          inputs.flake-parts.flakeModules.partitions
          ./rust/nci.nix
          ./doc/manual/flake-module.nix
        ];
        systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
        perSystem = { config, self', inputs', pkgs, ... }: {
          packages.default = pkgs.callPackage ./package.nix {
            nixops4-cli-rust = config.packages.nixops4-release;
            nixops4-eval = config.packages.nixops4-eval-release;
          };
          packages.nixops4-resource-runner = pkgs.callPackage ./rust/nixops4-resource-runner/package.nix { nixops4-resource-runner = config.packages.nixops4-resource-runner-release; };
          packages.nix = inputs'.nix.packages.nix;
          checks.json-schema = pkgs.callPackage ./test/json-schema.nix { };
          checks.nixops4-resources-local = pkgs.callPackage ./test/nixops4-resources-local.nix {
            inherit (config.packages) nixops4-resource-runner;
            nixops4-resources-local = config.packages.nixops4-resources-local-release;
          };

          /** A shell containing the packages of this flake. For development, use the `default` dev shell. */
          devShells.example = pkgs.mkShell {
            nativeBuildInputs = [
              config.packages.default
              config.packages.nixops4-resource-runner
            ];
          };
        };

        partitionedAttrs.devShells = "dev";
        partitionedAttrs.checks = "dev";
        partitionedAttrs.herculesCI = "dev";
        partitions.dev.extraInputsFlake = ./dev;
        partitions.dev.module = {
          imports = [ ./dev/flake-module.nix ];
        };
      });
}
