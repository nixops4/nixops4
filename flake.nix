{
  description = "NixOps4, deployment tool";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nix.url = "github:NixOS/nix";
    nix.inputs.nixpkgs.follows = "nixpkgs";
    nix-cargo-integration.url = "github:yusdacra/nix-cargo-integration";
    nix-cargo-integration.inputs.nixpkgs.follows = "nixpkgs";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nix-bindings-rust.url = "github:nixops4/nix-bindings-rust/0.1.0";
    nix-bindings-rust.inputs.flake-parts.follows = "flake-parts";
    nix-bindings-rust.inputs.nix-cargo-integration.follows = "nix-cargo-integration";
    nix-bindings-rust.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    inputs@{ self, flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } (
      {
        lib,
        withSystem,
        flake-parts-lib,
        ...
      }:
      {
        imports = [
          inputs.nix-cargo-integration.flakeModule
          inputs.flake-parts.flakeModules.partitions
          inputs.flake-parts.flakeModules.modules
          inputs.nix-bindings-rust.modules.flake.default
          ./rust/nci.nix
          ./doc/manual/flake-module.nix
          ./test/nixos/flake-module.nix
        ];
        systems = [
          "x86_64-linux"
          "aarch64-linux"
          "x86_64-darwin"
          "aarch64-darwin"
        ];
        perSystem =
          {
            config,
            self',
            inputs',
            pkgs,
            ...
          }:
          {
            packages.default = config.packages.nixops4;

            packages.nixops4 = pkgs.callPackage ./package.nix {
              nixops4-cli-rust = config.packages.nixops4-release;
              nixops4-eval = config.packages.nixops4-eval-release;
            };

            packages.nixops4-resource-runner = pkgs.callPackage ./rust/nixops4-resource-runner/package.nix {
              nixops4-resource-runner = config.packages.nixops4-resource-runner-release;
            };

            packages.flake-in-a-bottle = pkgs.callPackage ./nix/flake-in-a-bottle/package.nix {
              nixops4Flake = self;
            };

            packages.die = pkgs.writeScriptBin "die" ''
              #!${pkgs.runtimeShell}
              printf "%s" "$*" >&2
              exit 1
            '';

            packages.nix = config.nix-bindings-rust.nixPackage;

            nix-bindings-rust.nixPackage = inputs'.nix.packages.nix;

            checks.json-schema = pkgs.callPackage ./test/json-schema.nix { };
            checks.nixops4-resources-local = pkgs.callPackage ./test/nixops4-resources-local.nix {
              inherit (config.packages) nixops4-resource-runner die;
              nixops4-resources-local = config.packages.nixops4-resources-local-release;
            };
            checks.itest-nixops4-resources-local =
              pkgs.callPackage ./test/integration-test-nixops4-with-local/check.nix
                {
                  inherit (config.packages) nixops4 flake-in-a-bottle die;
                  inherit inputs;
                };

            /**
              A shell containing the packages of this flake. For development, use the `default` dev shell.
            */
            devShells.example = pkgs.mkShell {
              nativeBuildInputs = [
                config.packages.default
                config.packages.nixops4-resource-runner
              ];
            };
          };
        flake.lib = import ./nix/lib/lib.nix {
          inherit lib self;
          selfWithSystem = withSystem;
        };
        flake.modules.flake.default = flake-parts-lib.importApply ./nix/flake-parts/flake-parts.nix {
          inherit self;
        };
        flake.modules.nixops4Deployment.default = ./nix/deployment/base-modules.nix;
        flake.modules.nixops4Provider.local = flake-parts-lib.importApply ./nix/providers/local.nix {
          inherit withSystem;
        };

        partitionedAttrs.devShells = "dev";
        partitionedAttrs.checks = "dev";
        partitionedAttrs.tests = "dev"; # nix-unit
        partitionedAttrs.herculesCI = "dev";
        partitions.dev.extraInputsFlake = ./dev;
        partitions.dev.module = {
          imports = [ ./dev/flake-module.nix ];
        };
      }
    );
}
