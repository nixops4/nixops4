# Defines the `type` option and configures the resource from a provider type.
{
  config,
  lib,
  options,
  ...
}:
let
  inherit (lib) mkIf mkOption types;

in
{
  options = {
    type = mkOption {
      description = ''
        A resource type from the `providers` module argument.
      '';
      type = types.raw;
      example = lib.literalExpression ''
        providers.local.file
      '';
    };
  };

  config = mkIf options.type.isDefined {
    provider.executable = config.type.provider.executable;
    provider.args = config.type.provider.args;
    resourceType = config.type.type;
    inherit (config.type) outputsSkeleton requireState isOptionalInputName;
    inputs =
      { ... }:
      {
        imports = [ config.type.inputs ];
      };
    outputs =
      { ... }:
      {
        imports = [ config.type.outputs ];
      };
  };
}
