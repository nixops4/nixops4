{
  perSystem =
    {
      lib,
      config,
      pkgs,
      ...
    }:

    let
      nciBuildConfig = config.nix-bindings-rust.nciBuildConfig;
    in

    {
      # https://flake.parts/options/nix-cargo-integration
      nci.projects.nixops4-project = {
        path = ./.;
        depsDrvConfig = {
          imports = [ nciBuildConfig ];
        };
        drvConfig = {
          imports = [ nciBuildConfig ];
        };
        profiles = {
          dev.drvConfig.env.RUSTFLAGS = "-D warnings";
          release.runTests = true;
        };
      };
      nci.crates.nixops4-eval.drvConfig = {
        mkDerivation = {
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

            # Create nix.conf with experimental features enabled
            mkdir -p "$NIX_CONF_DIR"
            echo "experimental-features = flakes" > "$NIX_CONF_DIR/nix.conf"

            # Init ahead of time, because concurrent initialization is flaky
            ${
              # Not using nativeBuildInputs because this should (hopefully) be
              # the only place where we need a nix binary. Let's stay in control.
              config.nix-bindings-rust.nixPackage
            }/bin/nix-store --init

            echo "Store initialized."
          '';
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
