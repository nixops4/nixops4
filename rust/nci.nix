{
  perSystem = { pkgs, config, ... }: {
    # https://flake.parts/options/nix-cargo-integration
    nci.projects.nixops4.path = ./.;
  };
}
