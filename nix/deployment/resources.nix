{ lib, ... }:
let
  inherit (lib) mkOption types;

in
{
  options = {
    resources = mkOption {
      type =
        types.lazyAttrsOf
          (types.submoduleWith {
            class = "nixops4Resource";
            modules = [ ];
          });
      default = { };
      description = ''
        The resources to deploy.
      '';
    };
  };
}
