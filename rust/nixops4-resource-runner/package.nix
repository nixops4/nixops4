{
  # The plain rust package
  #
  # NOTE: we currently don't distinguish between the build and host package (ie
  #       for cross compilation) but considering that we need both in this final
  #       derivation, we should not merge the below behavior into the rust
  #       package as overrides.
  nixops4-resource-runner
, stdenv
}:

stdenv.mkDerivation {
  name = "nixops4-resource-runner";
  inherit (nixops4-resource-runner) version;
  dontUnpack = true;
  nativeBuildInputs = [
    nixops4-resource-runner
  ];
  buildPhase = ''
    nixops4-resource-runner generate-man > nixops4-resource-runner.1
  '';
  installPhase = ''
    mkdir -p $out/bin $out/share/man/man1
    cp ${nixops4-resource-runner}/bin/nixops4-resource-runner $out/bin
    cp nixops4-resource-runner.1 $out/share/man/man1/
  '';
}
