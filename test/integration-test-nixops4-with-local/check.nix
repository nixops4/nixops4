# Run with:
#   nix build .#checks.<system>.itest-nixops4-resources-local
{
  hello,
  jq,
  nixops4,
  runCommand,
  inputs,
  stdenv,
  nix,
  formats,
  flake-in-a-bottle,
  die,
}:

let
  preEval =
    let
      outputs = (import ./flake/flake.nix).outputs (
        inputs
        // {
          nixops4 = inputs.self;
          self = outputs; # not super accurate
          null = throw "Tried to access null input";
        }
      );
    in
    outputs.nixops4Deployments.myDeployment;
in
runCommand "itest-nixops4-with-local"
  {
    providers = preEval.getProviders {
      system = stdenv.hostPlatform.system;
    };
    nativeBuildInputs = [
      nixops4
      jq
      hello
      nix
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
    h2() {
      echo
      echo "$@"
      hr
    }

    h1 SETTING UP

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
    echo 'substituters =' >> $store_data/etc/nix.conf

    cp -r --no-preserve=mode ${./flake}/ work
    cd work
    substituteInPlace flake.nix \
      --replace-fail '@nixpkgs@' ${inputs.nixpkgs} \
      --replace-fail '@nixops4@' ${flake-in-a-bottle} \
      --replace-fail '@flake-parts@' ${inputs.flake-parts} \
      --replace-fail '@system@' ${stdenv.hostPlatform.system} \
      ;

    (
      set -x
      # cat -n flake.nix

      cp ${
        (formats.json { }).generate "binaries.json" {
          inherit (inputs.self.packages.${stdenv.hostPlatform.system}) nixops4-resources-local-release;
        }
      } binaries.json

      nix flake lock -vv

      nix eval .#nixops4Deployments.myDeployment._type --show-trace

      h1 BASIC STATELESS DEPLOYMENT

      nixops4 apply -v myDeployment --show-trace

      test -f file.txt
      [[ "Hallo wereld" == "$(cat file.txt)" ]]
      rm file.txt
    )

    h1 FAILING DEPLOYMENT
    (
      set -x
      (
        set +e;
        # 3>&1 etc: swap stderr and stdout
        nixops4 apply -v failingDeployment --show-trace 3>&1 1>&2 2>&3 | tee err.log
        [[ $? == 1 ]]
      )
      [[ ! -e file.txt ]]

      grep -F 'oh no, this and that failed' err.log
      grep -F 'Failed to create stateless resource hello' err.log
    )

    h1 STATEFUL DEPLOYMENT
    (
      set -x
      # Test stateful deployment
      echo "=== Testing stateful deployment ==="

      # First apply with version 1
      nixops4 apply -v statefulDeployment --show-trace

      # Check that files were created
      test -f initial-version.md
      test -f current-version.md
      test -f nixops4-state.json

      # Check content of initial version file
      grep -F "My initial version: 1" initial-version.md
      grep -F "We're now at version 1." current-version.md

      # Check state file exists and has content
      [[ -s nixops4-state.json ]]

      # Update currentVersion to 2 in the flake
      sed -i 's/currentVersion = 1;/currentVersion = 2;/' flake.nix

      # Apply again with version 2
      nixops4 apply -v statefulDeployment --show-trace

      # Check that initial version remains 1 (memo resource preserves initial state)
      grep -F "My initial version: 1" initial-version.md

      # But current version file should show 2
      grep -F "We're now at version 2." current-version.md

      # State file should still exist
      test -f nixops4-state.json
    )
    # Show state file contents
    h2 nixops4-state.json
    cat nixops4-state.json
    hr

    h2 "Test file recreation after deletion"
    (
      set -x
      # Backup state file to verify it remains identical
      cp nixops4-state.json nixops4-state.json.backup

      # Remove the generated files (but keep state file)
      rm -f initial-version.md current-version.md

      # Verify files are gone
      [[ ! -f initial-version.md ]]
      [[ ! -f current-version.md ]]

      # Apply again - should recreate the missing files
      nixops4 apply -v statefulDeployment --show-trace

      # Verify files were recreated with correct content
      test -f initial-version.md
      test -f current-version.md
      grep -F "My initial version: 1" initial-version.md
      grep -F "We're now at version 2." current-version.md

      # State should be identical - no changes should have occurred
      test -f nixops4-state.json
      diff nixops4-state.json nixops4-state.json.backup

      # Clean up backup
      rm nixops4-state.json.backup
    )

    (
      set -x
      # Clean up
      rm -f initial-version.md current-version.md nixops4-state.json
    )

    h1 SUCCESS
    touch $out
  ''
