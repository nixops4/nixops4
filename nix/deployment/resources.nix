{ resources, lib, resourceProviderSystem, ... }:
let
  inherit (lib) mkOption types;

  injectOutputs = { name, ... }: {
    outputs = resources.${name};
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
            specialArgs = {
              inherit resourceProviderSystem;
            };
          });
      default = { };
    };
  };
  config = { };
}
