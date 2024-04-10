{
  # local inputs
  nixops4-cli-rust
, nixops4-eval
, # nixpkgs
  makeBinaryWrapper
, stdenv
,
}:

stdenv.mkDerivation {
  pname = "nixops4";
  version = nixops4-cli-rust.version;

  src = null;
  dontUnpack = true;

  nativeBuildInputs = [
    makeBinaryWrapper
  ];

  buildPhase = ''
    makeWrapper ${nixops4-cli-rust}/bin/nixops4 nixops4 \
      --set _NIXOPS4_EVAL ${nixops4-eval}/bin/nixops4-eval \
      ;
  '';

  installPhase = ''
    mkdir -p $out/bin
    cp -r nixops4 $out/bin
  '';
}
