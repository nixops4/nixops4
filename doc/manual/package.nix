{ lib
, stdenv
, mdbook
,
}:
let
  inherit (lib) fileset;
in
stdenv.mkDerivation (finalAttrs: {
  name = "nixops-manual";

  src = fileset.toSource {
    fileset = fileset.unions [
      ./book.toml
      ./src
    ];
    root = ./.;
  };
  strictDeps = true;
  nativeBuildInputs = [
    mdbook
  ];
  buildPhase = ''
    runHook preBuild
    mdbook build
    runHook postBuild
  '';
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
