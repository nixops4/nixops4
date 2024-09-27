{ self }:
{ config, lib, withSystem, ... }:

let
  inherit (lib) mkOption types;
in
{
  options = {
    nixops4Deployments = mkOption {
      type =
        types.lazyAttrsOf
          (types.deferredModuleWith {
            staticModules = [ self.modules.nixops4Deployment.default ];
          });
      default = { };
    };
  };
  config = {
    perSystem = { pkgs, system, ... }: {
      checks =
        lib.concatMapAttrs
          (name: deployment:
            {
              "nixops-deployment-providers-${name}" =
                deployment.getProviders { inherit system; };
            }
          )
          config.flake.nixops4Deployments;
    };
    flake = {
      nixops4Deployments =
        lib.mapAttrs
          (name: module:
            self.lib.deployment.mkDeployment {
              modules = [
                ({ resourceProviderSystem, ... }: {
                  _file = __curPos.file;
                  _module.args.withResourceProviderSystem = lib.mkDefault (withSystem resourceProviderSystem);
                })
                module
              ];
              specialArgs = { };
              prefix = [ "nixops4Deployments" name ];
            }
          )
          config.nixops4Deployments;
    };
  };
}
