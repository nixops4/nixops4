{
  lib,
  inputs,
  self,
  withSystem,
  ...
}:
{
  imports = [
    inputs.pre-commit-hooks-nix.flakeModule
    inputs.hercules-ci-effects.flakeModule
    inputs.nix-unit.modules.flake.default
  ];
  perSystem =
    {
      config,
      pkgs,
      inputs',
      system,
      ...
    }:
    {

      nix-unit.tests = {
        lib = import ../nix/lib/tests.nix {
          inherit lib self system;
        };
        flake-parts = import ../nix/flake-parts/unit-tests.nix {
          flake-parts = inputs.flake-parts;
          nixops4 = self;
        };
      };
      nix-unit.inputs = {
        inherit (inputs) flake-parts nixpkgs nix-cargo-integration;
        "flake-parts/nixpkgs-lib" = inputs.flake-parts.inputs.nixpkgs-lib;
        "nix-cargo-integration/treefmt" = inputs.nix-cargo-integration.inputs.treefmt;
        "nix-bindings-rust" = inputs.nix-bindings-rust;
      };
      nix-unit.allowNetwork = true;

      pre-commit.settings.hooks.nixfmt.enable = true;
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

      devShells.default = pkgs.mkShell (
        {
          name = "nixops4-devshell";
          strictDeps = true;
          inputsFrom = [ config.nci.outputs.nixops4-project.devShell ];
          inherit (config.nci.outputs.nixops4-project.devShell.env) LIBCLANG_PATH;
          NIX_DEBUG_INFO_DIRS =
            let
              # TODO: add to Nixpkgs lib
              getDebug =
                pkg:
                if pkg ? debug then
                  pkg.debug
                else if pkg ? lib then
                  pkg.lib
                else
                  pkg;
            in
            "${getDebug config.nix-bindings-rust.nixPackage}/lib/debug";
          buildInputs = [
            config.nix-bindings-rust.nixPackage
          ];
          nativeBuildInputs = [
            pkgs.rust-analyzer
            pkgs.nixfmt
            pkgs.rustfmt
            pkgs.pkg-config
            pkgs.clang-tools # clangd
            pkgs.gdb
            pkgs.hci
            inputs'.nix-unit.packages.nix-unit
          ]
          ++ config.packages.manual.externalBuildTools;
          shellHook = ''
            ${config.pre-commit.shellHook}
            source ${inputs.nix-bindings-rust + "/bindgen-gcc.sh"}
            source ${../rust/artifact-shell.sh}
            echo 1>&2 "Welcome to the development shell!"
          '';
          # rust-analyzer needs a NIX_PATH for some reason
          NIX_PATH = "nixpkgs=${inputs.nixpkgs}";
        }
        // lib.optionalAttrs (config.nci.outputs.nixops4-project.devShell.env ? NIX_CC_UNWRAPPED) {
          inherit (config.nci.outputs.nixops4-project.devShell.env) NIX_CC_UNWRAPPED;
        }
      );
    };
  hercules-ci.flake-update = {
    enable = true;
    baseMerge.enable = true;
    autoMergeMethod = "merge";
    when = {
      dayOfMonth = 2;
    };
    flakes = {
      "." = { };
      "dev" = { };
    };
  };
  herculesCI =
    hci@{ config, primaryRepo, ... }:
    {
      ciSystems = [ "x86_64-linux" ];
      onPush.default.outputs = {
        effects.previewDocs = lib.optionalAttrs (hci.config.repo.branch != null) (
          withSystem "x86_64-linux" (
            perSystem@{
              config,
              hci-effects,
              pkgs,
              ...
            }:
            (hci-effects.netlifyDeploy {
              siteId = "f73012af-6a28-4dea-a0ea-5eee5cb56dd4";
              secretName = "netlify-nixops4-previews";
              extraDeployArgs = [
                "--alias"
                hci.config.repo.branch
              ];
              preEffect = ''
                git clone https://github.com/nixops4/nixops4.git --branch site-content --depth 1
                cd nixops4
                mkdir -p manual
                rm -rf manual/development .git
                cp -r ${perSystem.config.packages.manual.html} manual/development
                { echo 'User-agent: *'
                  echo 'Disallow: /'
                } >robots.txt
              '';
              content = ".";
            }).overrideAttrs
              (prevAttrs: {
                nativeBuildInputs = prevAttrs.nativeBuildInputs or [ ] ++ [ pkgs.git ];
              })
          )
        );
        effects.pushDocs = lib.optionalAttrs (hci.config.repo.branch == "main") (
          withSystem "x86_64-linux" (
            perSystem@{ config, hci-effects, ... }:
            hci-effects.gitWriteBranch {
              git.checkout.remote.url = hci.config.repo.remoteHttpUrl;
              git.checkout.forgeType = "github";
              git.checkout.user = "x-access-token";
              git.update.branch = "site-content";
              contents = perSystem.config.packages.manual.html;
              destination = "manual/development";
            }
          )
        );
      };
    };
  flake = { };
}
