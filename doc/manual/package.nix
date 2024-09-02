{ lib
, stdenv
, mdbook
, buildPackages
, nixops4-resource-runner
}:
let
  inherit (lib) fileset;
in
stdenv.mkDerivation (finalAttrs: {
  name = "nixops-manual";

  src = fileset.toSource {
    fileset = fileset.unions [
      ./Makefile
      ./book.toml
      ./src
      ./json-schema-for-humans-config.yaml
      ../../rust/nixops4-resource/resource-schema-v0.json
      ../../rust/nixops4-resource/examples
    ];
    root = ../..;
  };
  sourceRoot = "source/doc/manual";
  strictDeps = true;
  nativeBuildInputs = [
    mdbook
    buildPackages.python3Packages.json-schema-for-humans
    nixops4-resource-runner
  ];
  installPhase = ''
    runHook preInstall
    docDir="$out/share/doc/nixops4/manual"
    mkdir -p "$docDir"
    mv book/ "$docDir/html"
    runHook postInstall
  '';
  allowedReferences = [ ];

  passthru = {
    html = finalAttrs.finalPackage.out + "/share/doc/nixops4/manual/html";
  };
})
