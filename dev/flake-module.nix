top@{
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
    (
      # Expose the module arguments in config. We should probably have this in flake-parts and/or module system
      { config, specialArgs, ... }:
      {
        options.myModuleArguments = lib.mkOption { };
        config.myModuleArguments = config._module.args // specialArgs;
      }
    )
  ];

  # Create a partition with mocked packages for faster nix-unit testing
  # This provides a mocked self and selfWithSystem.
  partitions.unit-test-mock.module = {
    perSystem =
      { pkgs, ... }:
      {
        # nci uses something like mkDefault here, so we can override this with ease
        packages.nixops4-resources-terraform-release = pkgs.writeScriptBin "mock-nixops4-resources-terraform" "mock-binary";
      };
  };

  perSystem =
    {
      config,
      options,
      pkgs,
      inputs',
      system,
      ...
    }:
    {
      # Run with either:
      #   nix-unit --flake .#tests.systems.<system>
      # or, slower:
      #   nix build .#checks.<system>.nix-unit
      nix-unit.tests = {
        lib = import ../nix/lib/tests.nix {
          inherit lib self system;
        };
        flake-parts = import ../nix/flake-parts/unit-tests.nix {
          flake-parts = inputs.flake-parts;
          nixops4 = self;
        };
        tf-provider-to-module = import ../nix/tf-provider-to-module/tests/test.nix {
          inherit lib system;

          # Use partition's withSystem that has mocked packages
          selfWithSystem = top.config.partitions.unit-test-mock.module.myModuleArguments.withSystem;
          # .flake: note that this is not a complete flake yet with inputs, outputs, outPath, sourceInfo, ...
          self = top.config.partitions.unit-test-mock.module.flake;
        };
        tf-provider-to-module-real = lib.optionalAttrs (!options ? nciIsMocked) (
          import ../nix/tf-provider-to-module/tests/test.nix {
            inherit lib self system;
            selfWithSystem = withSystem;
          }
        );
      };
      nix-unit.inputs = {
        inherit (inputs) flake-parts nixpkgs;
        "flake-parts/nixpkgs-lib" = inputs.flake-parts.inputs.nixpkgs-lib;
        "nix-cargo-integration" = ./mock-nci;
      };
      nix-unit.allowNetwork = true;

      pre-commit.settings.hooks.nixfmt-rfc-style.enable = true;
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
          NIX_CC_UNWRAPPED
          ;
        inherit (config.nci.outputs.nixops4-project.devShell.env)
          _NIXOPS4_TEST_TERRAFORM_PROVIDER_LOCAL
          ;
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
          "${getDebug config.packages.nix}/lib/debug";
        buildInputs = [
          config.packages.nix
        ];
        nativeBuildInputs = [
          pkgs.rust-analyzer
          pkgs.nixfmt-rfc-style
          pkgs.rustfmt
          pkgs.pkg-config
          pkgs.clang-tools # clangd
          pkgs.valgrind
          pkgs.gdb
          pkgs.hci
          inputs'.nix-unit.packages.nix-unit
          pkgs.protobuf # For tonic-prost-build
          # TODO: set up cargo-valgrind in shell and build
          #       currently both this and `cargo install cargo-valgrind`
          #       produce a binary that says ENOENT.
          # pkgs.cargo-valgrind
        ]
        ++ config.packages.manual.externalBuildTools;
        shellHook = ''
          ${config.pre-commit.installationScript}
          source ${../rust/bindgen-gcc.sh}
          source ${../rust/artifact-shell.sh}
          echo 1>&2 "Welcome to the development shell!"
        '';
        # rust-analyzer needs a NIX_PATH for some reason
        NIX_PATH = "nixpkgs=${inputs.nixpkgs}";
      };
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
