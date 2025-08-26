{
  lib,
  resources,
  resourceProviderSystem,
  ...
}:
let
  inherit (lib) mkOption types;

  injectOutputs =
    { name, ... }:
    {
      outputs =
        { ... }:
        {
          config = resources.${name};
        };
    };
in
{
  options = {
    resources = mkOption {
      type = types.lazyAttrsOf (
        types.submoduleWith {
          class = "nixops4Resource";
          modules = [
            ./resource.nix
            injectOutputs
            {
              # Forward the resource provider system, so that individual
              # resource modules can use it to build things for the local
              # platform.
              _module.args.resourceProviderSystem = resourceProviderSystem;
            }
          ];
        }
      );
      default = { };
      description = ''
        The resources to deploy.
      '';
    };
  };
}
