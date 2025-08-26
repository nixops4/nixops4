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
    nixops4Deployments = mkOption {
      description = ''
        An attribute set of NixOps4 deployments.
        See [`Module Options`](https://nixops.dev/manual/development/modules/).

        Each deployment passed to [`mkDeployment`](https://nixops.dev/manual/development/lib/#mkDeployment).

        Definitions in `nixops4Deployments.<name>` give rise to the definitions:
        - `flake.nixops4Configurations.<name>` - the flake output attribute for NixOps4,
        - `perSystem.checks.nixops-deployment-providers-<name>` ([`perSystem.checks`](flake-parts.md#opt-perSystem.checks)), to make sure the deployment's resource providers are available on the supported flake systems - i.e. that the deployment can performed _from_ all [systems](flake-parts.md#opt-systems).
      '';
      type = types.lazyAttrsOf (
        types.deferredModuleWith {
          staticModules = [ self.modules.nixops4Deployment.default ];
        }
      );
      default = { };
    };
  };
  config = {
    perSystem =
      { pkgs, system, ... }:
      {
        checks = lib.concatMapAttrs (name: deployment: {
          "nixops-deployment-providers-${name}" = deployment.getProviders { inherit system; };
        }) config.flake.nixops4Deployments;
      };
    flake = {
      nixops4Deployments = lib.mapAttrs (
        name: module:
        self.lib.mkDeployment {
          modules = [
            (
              { resourceProviderSystem, ... }:
              {
                _file = ./flake-parts.nix;
                _module.args.withResourceProviderSystem = lib.mkDefault (withSystem resourceProviderSystem);
              }
            )
            module
          ];
          specialArgs = { };
          prefix = [
            "nixops4Deployments"
            name
          ];
        }
      ) config.nixops4Deployments;
    };
  };
}
