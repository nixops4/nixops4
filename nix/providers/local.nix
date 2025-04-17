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
      requireState = false;
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
    state_file = {
      requireState = false;
      inputs = {
        options = {
          name = mkOption {
            type = types.str;
          };
        };
      };
      outputs = {
        options = { };
      };
    };
    exec = {
      requireState = false;
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
    memo = {
      requireState = true;
      inputs = {
        options = {
          initialize_with = mkOption {
            # TODO: types.json?
            type = types.anything;
            description = ''
              The initial value of the memo.
              The memo is _not_ updated if this value changes in later versions of your deployment expression.
            '';
          };
        };
      };
      outputs = {
        options = {
          value = mkOption {
            type = types.anything;
            description = ''
              The value of the memo, which is the value of the `initialize_with` input *when the memo was created*.
            '';
          };
        };
      };
    };
  };
}
