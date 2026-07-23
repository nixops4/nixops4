# Integration test for nixops4.nix file loading
#
# Run with:
#   nix build .#checks.<system>.itest-nixops4-resources-local-file
{
  hello,
  jq,
  nixops4,
  runCommand,
  inputs,
  stdenv,
  formats,
  die,
  writeText,
}:
let
  system = stdenv.hostPlatform.system;

  preEval = inputs.self.lib.mkRoot {
    modules = [
      (
        { providers, ... }:
        {
          providers.local = inputs.self.modules.nixops4Provider.local;
        }
      )
    ];
  };

  # Evaluates the already locked nixops4 flake, providing
  # nixops4.lib.mkRoot and nixops4.modules.nixops4Provider.
  nixops4Deps = writeText "nixops4-deps.nix" ''
    let
      flakePartsSrc = ${inputs.flake-parts};
      flakePartsFlake = import (flakePartsSrc + "/flake.nix");
      flakePartsOutputs = flakePartsFlake.outputs {
        self = flakePartsOutputs;
        nixpkgs-lib = {
          outPath = ${inputs.nixpkgs};
          lib = import (${inputs.nixpkgs} + "/lib");
        };
      };
      flake-parts = flakePartsOutputs // { outPath = flakePartsSrc; };

      nixops4Src = ${inputs.self};
      nciStubFlake = import (nixops4Src + "/nix/flake-in-a-bottle/prebuilt-nix-cargo-integration/flake.nix");
      nciStub = nciStubFlake.outputs {
        self = nciStub;
        binaries = ${
          (formats.json { }).generate "binaries.json" {
            inherit (inputs.self.packages.${system}) nixops4-resources-local-release;
          }
        };
      };
      nullFlake = {
        outPath = nixops4Src + "/nix/flake-in-a-bottle/null";
        modules.flake.default = _: {};
      };

      nixops4Outputs = (import (nixops4Src + "/flake.nix")).outputs {
        self = nixops4Outputs;
        nixpkgs = { outPath = ${inputs.nixpkgs}; };
        inherit flake-parts;
        nix-cargo-integration = nciStub // {
          outPath = nixops4Src + "/nix/flake-in-a-bottle/prebuilt-nix-cargo-integration";
        };
        nix = nullFlake;
        nix-bindings-rust = nullFlake;
        hercules-ci-effects = nullFlake // { flakeModule = _: {}; };
        pre-commit-hooks-nix = nullFlake // { flakeModule = _: {}; };
        nix-unit = nullFlake // { flakeModule = _: {}; };
      };
    in
      nixops4Outputs
  '';

  # nixops4.nix that defines the deployment directly using mkRoot.
  nixops4File =
    greeting:
    writeText "nixops4-itest.nix" ''
      let
        nixops4 = import ./nixops4-deps.nix;
      in
        nixops4.lib.mkRoot {
          modules = [
            ({ providers, members, ... }: {
              providers.local = nixops4.modules.nixops4Provider.local;
              members.myDeployment = {
                members.hello = {
                  type = providers.local.exec;
                  inputs = {
                    executable = "hello";
                    args = [ "--greeting" "${greeting}" ];
                  };
                };
                members."file.txt" = {
                  type = providers.local.file;
                  inputs = {
                    name = "file.txt";
                    contents = members.myDeployment.members.hello.outputs.stdout;
                  };
                };
              };
            })
          ];
        }
    '';
in
runCommand "itest-nixops4-with-local-file"
  {
    providers = preEval.getProviders { inherit system; };
    nativeBuildInputs = [
      nixops4
      jq
      hello
      die
    ];
  }
  ''
    hr() {
      echo -----------------------------------------------------------------------
    }
    h1() {
      echo
      hr
      echo "$@"
      hr
    }

    h1 SETTING UP

    export HOME=$(mktemp -d $TMPDIR/home.XXXXXX)

    # nixops4.nix must be picked regardless of flake.nix presence or
    # experimental-features settings. We enable flakes and place a
    # flake.nix guard so that flake evaluation is a viable path —
    # proving nixops4.nix is chosen over it, not just the only option.
    export NIX_CONF_DIR=$TMPDIR/etc
    mkdir -p $NIX_CONF_DIR
    echo 'extra-experimental-features = flakes' > $NIX_CONF_DIR/nix.conf
    echo 'substituters =' >> $NIX_CONF_DIR/nix.conf

    mkdir work
    cd work

    cp --no-preserve=mode ${nixops4Deps} nixops4-deps.nix
    cp --no-preserve=mode ${nixops4File "Hello from nixops4.nix"} nixops4.nix

    # Sanity check: verify flake evaluation is reachable in this
    # environment, so the tests below prove a real choice was made.
    h1 "SANITY CHECK: flake evaluation is reachable"
    (
      echo '{ outputs = _: abort "sanity check: flake.nix was evaluated"; }' > flake.nix
      mv nixops4.nix nixops4.nix.bak
      (
        set +e
        nixops4 apply myDeployment > flake-viable.stdout 2> flake-viable.stderr
        [[ $? != 0 ]]
      )
      [[ ! -s flake-viable.stdout ]]
      grep -F 'sanity check: flake.nix was evaluated' flake-viable.stderr
      mv nixops4.nix.bak nixops4.nix
    )

    # Guard: if nixops4.nix discovery were bypassed, this aborts.
    echo '{ outputs = _: abort "flake.nix must not be used when nixops4.nix is present"; }' > flake.nix

    h1 BASIC STATELESS DEPLOYMENT
    (
      set -x
      nixops4 apply -v myDeployment --show-trace

      test -f file.txt
      # Greeting proves nixops4.nix was used, not flake discovery
      [[ "Hello from nixops4.nix" == "$(cat file.txt)" ]]
      rm file.txt
    )

    h1 NO FALLBACK TO FLAKE
    (
      set -x
      # Overwriting nixops4.nix with an invalid expression must
      # produce a type error, not silently fall back to flake discovery.
      echo '{ }' > nixops4.nix
      (
        set +e
        nixops4 apply myDeployment > no-fallback.stdout 2> no-fallback.stderr
        [[ $? != 0 ]]
      )
      [[ ! -s no-fallback.stdout ]]
      grep -F 'attribute `_type` not found' no-fallback.stderr

      # Restore working nixops4.nix
      cp ${nixops4File "Hello from nixops4.nix"} nixops4.nix
    )

    h1 FLAKE OPTIONS CONFLICT WITH NIXOPS4.NIX
    (
      set -x
      # nixops4.nix is present, so --override-input must be rejected
      (
        set +e
        nixops4 apply --override-input nixpkgs /dev/null myDeployment > conflict.stdout 2> conflict.stderr
        [[ $? != 0 ]]
      )
      [[ ! -s conflict.stdout ]]
      grep -F 'nixops4.nix found in current directory' conflict.stderr
    )

    # Create explicit-file.nix with a distinct greeting.
    # nixops4.nix ("Hello from nixops4.nix") stays in place so we can
    # prove --file takes priority through the output content.
    cp --no-preserve=mode ${nixops4File "Hello from --file"} explicit-file.nix

    h1 EXPLICIT --file TAKES PRIORITY OVER NIXOPS4.NIX
    (
      set -x
      # nixops4.nix is still present with "Hello from nixops4.nix".
      # The distinct greeting proves --file was used instead.
      nixops4 apply -v --file explicit-file.nix myDeployment --show-trace

      test -f file.txt
      [[ "Hello from --file" == "$(cat file.txt)" ]]
      rm file.txt
    )

    h1 --file FILE NOT FOUND
    (
      set -x
      (
        set +e
        nixops4 apply --file /nonexistent/path.nix myDeployment > notfound.stdout 2> notfound.stderr
        [[ $? != 0 ]]
      )
      [[ ! -s notfound.stdout ]]
      # Nix inserts ANSI codes around the path, so match the surrounding text
      grep "path.*nonexistent/path.nix.*does not exist" notfound.stderr
    )

    h1 --file SYNTAX ERROR
    (
      set -x
      echo 'this is not { valid nix' > bad-syntax.nix
      (
        set +e
        nixops4 apply --file bad-syntax.nix myDeployment > syntax.stdout 2> syntax.stderr
        [[ $? != 0 ]]
      )
      [[ ! -s syntax.stdout ]]
      grep -F 'syntax error, unexpected' syntax.stderr
    )

    h1 --file WRONG TYPE
    (
      set -x
      echo '{ }' > wrong-type.nix
      (
        set +e
        nixops4 apply --file wrong-type.nix myDeployment > type.stdout 2> type.stderr
        [[ $? != 0 ]]
      )
      [[ ! -s type.stdout ]]
      grep -F 'attribute `_type` not found' type.stderr
    )

    h1 --file FLAKE OPTIONS CONFLICT
    (
      set -x
      (
        set +e
        # clap should reject this at parse time
        nixops4 apply --file explicit-file.nix --override-input nixpkgs /dev/null myDeployment > file-conflict.stdout 2> file-conflict.stderr
        [[ $? != 0 ]]
      )
      [[ ! -s file-conflict.stdout ]]
      grep -F "cannot be used with '--override-input" file-conflict.stderr
    )

    h1 SUCCESS
    touch $out
  ''
