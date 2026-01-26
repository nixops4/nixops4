{ self }:
{
  config,
  lib,
  withSystem,
  ...
}:

let
  inherit (lib) mkOption types;
in
{
  options = {
    nixops4 = mkOption {
      description = ''
        The NixOps4 root component configuration.
        See [`Module Options`](https://nixops.dev/manual/development/modules/).

        This module is passed to [`mkRoot`](https://nixops.dev/manual/development/lib/#mkRoot).

        Defining `nixops4` gives rise to the definitions:
        - `flake.nixops4` - the flake output attribute for NixOps4,
        - `perSystem.checks.nixops-providers` ([`perSystem.checks`](flake-parts.md#opt-perSystem.checks)), to make sure the root's resource providers are available on the supported flake systems - i.e. that operations can be performed _from_ all [systems](flake-parts.md#opt-systems).
      '';
      type = types.deferredModuleWith {
        staticModules = [ self.modules.nixops4Component.default ];
      };
      default = { };
    };
  };
  config = {
    perSystem =
      { pkgs, system, ... }:
      {
        checks = {
          nixops-providers = config.flake.nixops4.getProviders { inherit system; };
        };
      };
    flake = {
      nixops4 = self.lib.mkRoot {
        modules = [
          (
            { resourceProviderSystem, ... }:
            {
              _file = ./flake-parts.nix;
              _module.args.withResourceProviderSystem = lib.mkDefault (withSystem resourceProviderSystem);
            }
          )
          config.nixops4
        ];
        specialArgs = { };
        prefix = [ "nixops4" ];
      };
    };
  };
}
