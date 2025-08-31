{
  perSystem =
    {
      lib,
      config,
      pkgs,
      ...
    }:
    {
      # https://flake.parts/options/nix-cargo-integration
      nci.projects.nixops4-project = {
        path = ./.;
        drvConfig = {
          mkDerivation = {
            buildInputs = [
              # stdbool.h
              pkgs.stdenv.cc
            ]
            ++ (
              if config.packages.nix ? libs then
                let
                  l = config.packages.nix.libs;
                in
                [
                  l.nix-expr-c
                  l.nix-store-c
                  l.nix-util-c
                  l.nix-flake-c
                ]
              else
                [ config.packages.nix ]
            );
            nativeBuildInputs = [
              pkgs.pkg-config
            ];
            # bindgen uses clang to generate bindings, but it doesn't know where to
            # find our stdenv cc's headers, so when it's gcc, we need to tell it.
            postConfigure = lib.optionalString pkgs.stdenv.cc.isGNU ''
              source ${./bindgen-gcc.sh}
            '';
            # Prepare the environment for Nix to work.
            # Nix does not provide a suitable environment for running itself in
            # the sandbox - not by default. We configure it to use a relocated store.
            preCheck = ''
              # nix needs a home directory
              export HOME="$(mktemp -d $TMPDIR/home.XXXXXX)"

              # configure a relocated store
              store_data=$(mktemp -d $TMPDIR/store-data.XXXXXX)
              export NIX_REMOTE="$store_data"
              export NIX_BUILD_HOOK=
              export NIX_CONF_DIR=$store_data/etc
              export NIX_LOCALSTATE_DIR=$store_data/nix/var
              export NIX_LOG_DIR=$store_data/nix/var/log/nix
              export NIX_STATE_DIR=$store_data/nix/var/nix

              echo "Configuring relocated store at $NIX_REMOTE..."

              # Init ahead of time, because concurrent initialization is flaky
              ${
                # Not using nativeBuildInputs because this should (hopefully) be
                # the only place where we need a nix binary. Let's stay in control.
                pkgs.buildPackages.nix
              }/bin/nix-store --init

              echo "Store initialized."
            '';
          };
          # NOTE: duplicated in flake.nix devShell
          env = {
            LIBCLANG_PATH = lib.makeLibraryPath [ pkgs.buildPackages.llvmPackages.clang-unwrapped ];
            BINDGEN_EXTRA_CLANG_ARGS =
              # Work around missing [[deprecated]] in clang
              "-x c++ -std=c++2a";
          }
          // lib.optionalAttrs pkgs.stdenv.cc.isGNU {
            # Avoid cc wrapper, because we only need to add the compiler/"system" dirs
            NIX_CC_UNWRAPPED = "${pkgs.stdenv.cc.cc}/bin/gcc";
          };
        };
      };

      nci.crates.nixops4-resources-terraform.drvConfig = {
        env =
          let
            # TODO:
            #  - figure out how Nixpkgs achieves a locked terraform command
            #  - is that the right way to go about things for us?
            #  - adopt this elsewhere, drop it, and/or accept this as an ad hoc testing solution
            providerPath =
              pkg:
              "${pkg}/libexec/terraform-providers/${pkg.provider-source-address}/${pkg.version}/${pkg.GOOS}_${pkg.GOARCH}/${pkg.pname}_${pkg.version}";
          in
          {
            _NIXOPS4_TEST_TERRAFORM_PROVIDER_LOCAL = providerPath pkgs.terraform-providers.local;
          };
        mkDerivation = {
          # Should be nativeCheckInputs, but that doesn't seem wired up in nix-cargo-integration or its dependencies (dream2nix)
          nativeBuildInputs = [ pkgs.protobuf ];
          meta.mainProgram = "nixops4-resources-terraform";
        };
      };

    };
}
