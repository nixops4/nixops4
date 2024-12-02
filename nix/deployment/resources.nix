{ lib, resources, ... }:
let
  inherit (lib) mkOption types;

  injectOutputs = { name, ... }: {
    outputs = { ... }: {
      config = resources.${name};
    };
  };
in
{
  options = {
    resources = mkOption {
      type =
        types.lazyAttrsOf
          (types.submoduleWith {
            class = "nixops4Resource";
            modules = [
              ./resource.nix
              injectOutputs
            ];
          });
      default = { };
      description = ''
        The resources to deploy.
      '';
    };
  };
}
