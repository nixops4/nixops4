thisFlake@{ withSystem }:

# The actual module
{ lib, resourceProviderSystem, withSystem, ... }:
let
  inherit (lib) mkOption types;
in
{
  executable =
    thisFlake.withSystem resourceProviderSystem ({ config, ... }:
      "${config.packages.nixops4-resources-local-release}/bin/nixops4-resources-local"
    );
  resourceTypes = {
    file = {
      inputs = {
        options = {
          name = mkOption {
            type = types.str;
          };
          contents = mkOption {
            type = types.str;
          };
        };
      };
      outputs = {
        options = { };
      };
    };
    exec = {
      inputs = {
        options = {
          executable = mkOption {
            type = types.coercedTo (types.functionTo types.str) (withSystem resourceProviderSystem) types.str;
          };
          args = mkOption {
            type = types.listOf (types.coercedTo (types.oneOf [ types.str types.path types.int ]) (x: "${x}") types.str);
            default = [ ];
          };
          stdin = mkOption {
            type = types.nullOr types.str;
            default = null;
          };
        };
      };
      outputs = {
        options = {
          stdout = mkOption {
            type = types.str;
          };
        };
      };
    };
  };
}
