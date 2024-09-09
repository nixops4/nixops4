{ lib, config, ... }:
let
  inherit (lib) mkOption types;
in
{
  options = {
    type = mkOption {
      type = types.raw;
      example = lib.literalExpression ''
        providers.local.file
      '';
    };
    provider.command = mkOption {
      type = types.str;
      description = ''
        The path to the executable that implements the resource operations.
      '';
    };
    provider.args = mkOption {
      type = types.listOf types.str;
      default = [ ];
      description = ''
        The arguments to pass to the executable.
      '';
    };
    provider.type = mkOption {
      type = types.enum [ "stdio" ];
      default = "stdio";
      description = ''
        The type of communication to use with the executable.
      '';
    };
    provider.types = mkOption {
      type = types.lazyAttrsOf types.raw;
      description = ''
        The types of resources that the provider can manage.
      '';
    };
    inputs = mkOption {
      type = types.submodule { };
    };
    outputs = mkOption {
      type = types.submodule { };
    };
  };
  config = {
    provider.command = config.type.provider.command;
    provider.args = config.type.provider.args;
    provider.types = config.type.provider.types;
    inputs = { ... }: { imports = [ config.type.inputs ]; };
    outputs = { ... }: { imports = [ config.type.outputs ]; };
  };
}
