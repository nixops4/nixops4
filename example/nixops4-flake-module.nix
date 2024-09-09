{ config, lib, ... }:

let
  inherit (lib) mkOption types;

  makeBaseArgs =
    { name, module }:
    {
      prefix = [ "nixops4Deployments" name ];
      modules = [
        module
      ];
      specialArgs = { };
    };

  evalDeployment =
    baseArgs:
    { resources, ... }:
    let
      conf =
        lib.evalModules (baseArgs // {
          specialArgs = baseArgs.specialArgs // {
            inherit resources;
            # FIXME
            resourceProviderSystem = "x86_64-linux";
          };
        });
    in
    conf // {
      resources =
        lib.mapAttrs
          (name: resource: resource // {
            type = resource.type.type;
          })
          conf.config.resources;
    };

  evalDeploymentForProviders =
    baseArgs:
    { system, ... }:
    evalDeployment
      baseArgs
      {
        resources = throw "resource information is not available when evaluating a deployment for the purpose of building the providers ahead of time.";
      };

in
{
  options = {
    nixops4Deployments = mkOption {
      type =
        types.lazyAttrsOf
          (types.deferredModuleWith {
            staticModules = [ ./nixops4-modules.nix ];
          });
      default = { };
    };
  };
  config = {
    perSystem = { pkgs, system, ... }: {
      checks =
        lib.concatMapAttrs
          (name: module:
            {
              "nixops-deployment-providers-${name}" =
                let
                  conf =
                    evalDeploymentForProviders
                      (makeBaseArgs { inherit name module; })
                      { inherit system; };
                  serializable =
                    lib.mapAttrs
                      (name: provider:
                        {
                          command = provider.command;
                          args = provider.args;
                        }
                      )
                      conf.config.providers;
                in
                (pkgs.writeText "nixops-deployment-providers-${name}"
                  (builtins.toJSON serializable
                  )).overrideAttrs {
                  passthru.config = conf.config;
                };
            }
          )
          config.nixops4Deployments;
    };
    flake = {
      nixops4Deployments =
        lib.mapAttrs
          (name: module:
            {
              _type = "nixops4Deployment";
              deploymentFunction =
                let
                  baseArgs = makeBaseArgs { inherit name module; };
                in
                { resources, ... }:
                let
                  eval =
                    evalDeployment
                      (baseArgs // {
                        specialArgs = baseArgs.specialArgs // {
                          inherit resources;
                          # FIXME
                          resourceProviderSystem = "x86_64-linux";
                        };
                      })
                      { inherit resources; };
                in
                { inherit (eval) resources; };
            }
          )
          config.nixops4Deployments;
    };
  };
}
