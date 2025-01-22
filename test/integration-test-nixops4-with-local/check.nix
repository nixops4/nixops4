{ hello
, jq
, nixops4
, runCommand
, inputs
, stdenv
, nix
, formats
}:

let
  preEval =
    let
      outputs =
        (import ./flake/flake.nix).outputs
          (inputs // {
            nixops4 = inputs.self;
            self = outputs; # not super accurate
            null = throw "Tried to access null input";
          });
    in
    outputs.nixops4Deployments.myDeployment;
in
runCommand
  "itest-nixops4-with-local"
{
  providers = preEval.getProviders {
    system = stdenv.hostPlatform.system;
  };
  nativeBuildInputs = [
    nixops4
    jq
    hello
    nix
  ];
}
  ''
    export HOME=$(mktemp -d $TMPDIR/home.XXXXXX)
    # configure a relocated store
    store_data=$(mktemp -d $TMPDIR/store-data.XXXXXX)
    export NIX_REMOTE="$store_data"
    # export NIX_STORE="/nix/store?root=$store_data"
    # export NIX_STORE="/nix/store?real=$store_data"
    export NIX_BUILD_HOOK=
    export NIX_CONF_DIR=$store_data/etc
    export NIX_LOCALSTATE_DIR=$store_data/nix/var
    export NIX_LOG_DIR=$store_data/nix/var/log/nix
    export NIX_STATE_DIR=$store_data/nix/var/nix

    mkdir -p $NIX_CONF_DIR
    echo 'extra-experimental-features = flakes nix-command' > $store_data/etc/nix.conf

    cp -r --no-preserve=mode ${./flake}/ work
    cd work
    substituteInPlace flake.nix \
      --replace-fail '@nixpkgs@' ${inputs.nixpkgs} \
      --replace-fail '@nixops4@' ${inputs.self} \
      --replace-fail '@flake-parts@' ${inputs.flake-parts} \
      --replace-fail '@system@' ${stdenv.hostPlatform.system} \
      --replace-fail '@prebuilt-nix-cargo-integration@' ${../prebuilt-nix-cargo-integration} \
      --replace-fail '@null@' ${../null}

    (
      set -x;
      # cat -n flake.nix

      cp ${(formats.json {}).generate "binaries.json" {
        inherit (inputs.self.packages.${stdenv.hostPlatform.system}) nixops4-resources-local-release;
      }} binaries.json

      # workaround for https://github.com/NixOS/nix/issues/12040
      nix store add --name source --hash-algo sha256 --mode nar ${../prebuilt-nix-cargo-integration}

      nix flake lock -vv

      nix eval .#nixops4Deployments.myDeployment._type --show-trace

      nixops4 apply -v myDeployment --show-trace

      test -f file.txt
      [[ "Hallo wereld" == "$(cat file.txt)" ]]

      touch $out
    )
  ''
