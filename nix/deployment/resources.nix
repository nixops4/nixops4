{ resources, lib, ... }:
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
            modules = [
              ./resource.nix
              injectOutputs
            ];
          });
      default = { };
    };
  };
  config = { };
}
