{ buildPackages
, cargo
, jq
, lib
, mdbook
, mdbook-mermaid
, nixops4-resource-runner
, stdenv
}:
let
  inherit (lib) fileset;
in
stdenv.mkDerivation (finalAttrs: {
  name = "nixops-manual";

  src = fileset.toSource {
    fileset = fileset.unions [
      ../../rust/nixops4-resource/examples
      ../../rust/nixops4-resource/resource-schema-v0.json
      (fileset.fileFilter ({ name, ... }: name == "Cargo.toml") ../../rust)
      ./book.toml
      ./cargo-deps.sh
      ./custom.css
      ./json-schema-for-humans-config.yaml
      ./make
      ./Makefile
      ./mermaid-init.js
      ./mermaid.min.js
      ./src
    ];
    root = ../..;
  };
  sourceRoot = "source/doc/manual";
  strictDeps = true;
  nativeBuildInputs = finalAttrs.passthru.externalBuildTools ++ [
    # cargo for the `cargo-deps.sh` script. Not listed in externalBuildTools because the shell already has it
    cargo
    nixops4-resource-runner
  ];
  preConfigure = ''
    patchShebangs --build ./cargo-deps.sh
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
    /** To add to the project-wide dev shell */
    externalBuildTools = [
      mdbook
      mdbook-mermaid
      buildPackages.python3Packages.json-schema-for-humans
      jq
    ];
  };
})
