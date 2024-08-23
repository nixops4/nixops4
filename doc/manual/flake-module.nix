{
  perSystem = { config, pkgs, ... }: {
    packages.manual = pkgs.callPackage ./package.nix { };
    checks.manual-links = pkgs.callPackage ./test.nix { site = config.packages.manual; };
  };
}
