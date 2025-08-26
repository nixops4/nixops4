{
  perSystem =
    { config, pkgs, ... }:
    let
      baseModule = {
        _file = "test/nixos/flake-module.nix#baseModule";
        nodes.deployer =
          { pkgs, ... }:
          {
            # System installation is not actually needed. Should we test without it?
            environment.systemPackages = [ config.packages.nixops4 ];
            nix.settings.experimental-features = "flakes";
          };
      };
    in
    {
      config = {
        checks.test-nixos = pkgs.testers.runNixOSTest {
          imports = [
            baseModule
            ./test.nix
          ];
        };
      };
    };
}
