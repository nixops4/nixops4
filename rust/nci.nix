{
  perSystem = { lib, config, pkgs, ... }: {
    # https://flake.parts/options/nix-cargo-integration
    nci.projects.nixops4-project = {
      path = ./.;
      drvConfig = {
        mkDerivation = {
          buildInputs = [
            config.packages.nix
            # stdbool.h
            pkgs.stdenv.cc
          ];
          nativeBuildInputs = [
            pkgs.pkg-config
          ];
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
            ${# Not using nativeBuildInputs because this should (hopefully) be
              # the only place where we need a nix binary. Let's stay in control.
              pkgs.buildPackages.nix}/bin/nix-store --init

            echo "Store initialized."
          '';
        };
        # NOTE: duplicated in flake.nix devShell
        env = {
          LIBCLANG_PATH =
            if pkgs.stdenv.cc.isClang then
              null # don't set the variable
            else
              lib.makeLibraryPath [ pkgs.buildPackages.llvmPackages.clang-unwrapped ];
          BINDGEN_EXTRA_CLANG_ARGS =
            if pkgs.stdenv.cc.isClang then
              null # don't set the variable
            else
              "-I${pkgs.stdenv.cc.libc.dev}/include"
              + " -I${lib.getDev pkgs.stdenv.cc.cc}/lib/gcc/${pkgs.stdenv.hostPlatform.config}/${pkgs.stdenv.cc.cc.version}/include"
          ;
        };
      };
    };
  };
}
