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
              "-I${lib.getDev pkgs.stdenv.cc.cc}/lib/gcc/${pkgs.stdenv.hostPlatform.config}/${pkgs.stdenv.cc.cc.version}/include";
        };
      };
    };
  };
}
