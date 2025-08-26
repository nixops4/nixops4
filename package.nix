{
  # local inputs
  nixops4-cli-rust,
  nixops4-eval,
  # nixpkgs
  makeBinaryWrapper,
  stdenv,
}:

stdenv.mkDerivation {
  pname = "nixops4";
  version = nixops4-cli-rust.version;

  src = null;
  dontUnpack = true;

  nativeBuildInputs = [
    makeBinaryWrapper

    # Generated completions, man page
    nixops4-cli-rust
  ];

  buildPhase = ''
    makeWrapper ${nixops4-cli-rust}/bin/nixops4 nixops4 \
      --set _NIXOPS4_EVAL ${nixops4-eval}/bin/nixops4-eval \
      ;

    nixops4 generate-man > nixops4.1
    nixops4 generate-completion --shell bash > completion.bash
    nixops4 generate-completion --shell zsh > completion.zsh 
  '';

  installPhase = ''
    mkdir -p $out/bin
    cp -r nixops4 $out/bin

    mkdir -p $out/share/man/man1 $out/share/bash-completion/completions \
      $out/share/zsh/site-functions
    cp nixops4.1 $out/share/man/man1/
    cp completion.bash $out/share/bash-completion/completions/nixops4
    cp completion.zsh $out/share/zsh/site-functions/_nixops4
  '';
}
