{
  perSystem = { config, pkgs, ... }: {
    packages.manual = pkgs.callPackage ./package.nix {
      inherit (config.packages) nixops4-resource-runner;
    };
    checks.manual-links = pkgs.callPackage ./test.nix { site = config.packages.manual; };
  };
}
