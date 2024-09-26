# This would normally be the flake output function body scope, but we don't have
# a separate flake for the provider yet. Multi-flake development doesn't work
# quite right yet in Nix...
thisFlake@{ withSystem }:

# The actual module
{ lib, resourceProviderSystem, withSystem, ... }:
let
  inherit (lib) mkOption types;
in
{
  command =
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
          content = mkOption {
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
          command = mkOption {
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
