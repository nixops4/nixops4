{ config, lib, ... }:
let
  inherit (lib) mkOption types;
in
{
  imports = [
    ./provider-doc.nix
  ];
  _class = "nixops4Provider";
  options = {
    executable = mkOption {
      description = ''
        The path to the executable that implements the resource operations.
      '';
      type = types.str;
    };

    args = mkOption {
      description = ''
        Any command line arguments to pass to the executable.
      '';
      type = types.listOf types.str;
      default = [ ];
    };

    type = mkOption {
      description = ''
        The type of communication to use with the resource provider executable.
      '';
      type = types.str;
      default = "stdio";
    };

    resourceTypes = mkOption {
      description = ''
        The types of resources that this provider can create.

        The purpose of the `resourceTypes` option is to provide the information necessary to create the `providers` module argument.

        The attribute name under `resourceTypes` is the resource type, and gives rise to `providers.<provider>.<resourceType>`.
      '';
      type = types.lazyAttrsOf (types.submoduleWith {
        specialArgs.provider = config;
        class = "nixops4ResourceType";
        modules = [
          ../resourceType/resourceType.nix
          ({ name, ... }: { type = name; })
        ];
      });
    };
  };
}
