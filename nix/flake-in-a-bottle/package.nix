{
  runCommand,
  nixops4Flake,
  stdenv,
  formats,
  nix,
}:
let
  inputs = nixops4Flake.inputs;
  binaries = {
    inherit (nixops4Flake.packages.${stdenv.hostPlatform.system}) nixops4-resources-local-release;
  };
in
runCommand "nixops4-flake-in-a-bottle"
  {
    nativeBuildInputs = [ nix ];
    meta.description = "The nixops4 flake, but suitable for offline use";
    meta.longDescription = ''
      This is a flake that is suitable for "offline" use in the Nix build sandbox.
      This is therefore particularly useful for testing.

      Such functionality is reminiscent of `nix flake archive`, but that command expects to run in a networked environment, whereas this is implemented in the Nix language.
      Additionally, it can pre-build packages, so that this doesn't have to be done during the test.
    '';
  }
  ''
    mkdir work store home
    store=$PWD/store
    export HOME=$PWD/home
    cd work
    cp --no-preserve=mode ${./src/flake.nix} ./flake.nix
    substituteInPlace flake.nix \
      --replace-fail '@nixpkgs@' ${inputs.nixpkgs} \
      --replace-fail '@nixops4@' ${nixops4Flake} \
      --replace-fail '@flake-parts@' ${inputs.flake-parts} \
      --replace-fail '@prebuilt-nix-cargo-integration@' ${./prebuilt-nix-cargo-integration} \
      --replace-fail '@null@' ${./null} \
      --replace-fail '@binaries@' ${(formats.json { }).generate "binaries.json" binaries} \
      ;
    (
      set -x;

      nix() {
        command nix --experimental-features 'nix-command flakes' --store "$store" "$@"
      }

      nix flake lock -vv

      # error: 'lastModified' attribute mismatch in input 'path:/nix/store/...?lastModified=0&...', expected 1
      sed -i -e 's|"lastModified": 1,||' flake.lock

      mkdir $out
      cp flake.nix flake.lock $out/
    )
  ''
