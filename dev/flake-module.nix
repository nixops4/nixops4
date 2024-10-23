{ lib, inputs, withSystem, ... }: {
  imports = [
    inputs.pre-commit-hooks-nix.flakeModule
    inputs.hercules-ci-effects.flakeModule
  ];
  perSystem = { config, pkgs, ... }: {

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
        pkgs.hci
        # TODO: set up cargo-valgrind in shell and build
        #       currently both this and `cargo install cargo-valgrind`
        #       produce a binary that says ENOENT.
        # pkgs.cargo-valgrind
      ] ++ config.packages.manual.externalBuildTools;
      shellHook = ''
        ${config.pre-commit.installationScript}
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
    when = { dayOfMonth = 2; };
    flakes = {
      "." = { };
      "dev" = { };
    };
  };
  herculesCI = hci@{ config, primaryRepo, ... }: {
    ciSystems = [ "x86_64-linux" ];
    onPush.default.outputs = {
      effects.previewDocs =
        lib.optionalAttrs
          (hci.config.repo.branch != null)
          (withSystem "x86_64-linux" (perSystem@{ config, hci-effects, pkgs, ... }:
            (hci-effects.netlifyDeploy {
              siteId = "f73012af-6a28-4dea-a0ea-5eee5cb56dd4";
              secretName = "netlify-nixops4-previews";
              extraDeployArgs = [ "--alias" hci.config.repo.branch ];
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
            }).overrideAttrs (prevAttrs: {
              nativeBuildInputs = prevAttrs.nativeBuildInputs or [ ] ++ [ pkgs.git ];
            })
          ));
      effects.pushDocs =
        lib.optionalAttrs
          (hci.config.repo.branch == "main")
          (withSystem "x86_64-linux" (perSystem@{ config, hci-effects, ... }:
            hci-effects.gitWriteBranch {
              git.checkout.remote.url = hci.config.repo.remoteHttpUrl;
              git.checkout.forgeType = "github";
              git.checkout.user = "x-access-token";
              git.update.branch = "site-content";
              contents = perSystem.config.packages.manual.html;
              destination = "manual/development";
            }
          ));
    };
  };
  flake = { };
}
