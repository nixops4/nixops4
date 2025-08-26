thisFlake@{ withSystem }:

# The actual module
{ lib
, resourceProviderSystem
, withSystem
, ...
}:
let
  inherit (lib) mkOption types;
in
{
  name = "Local Provider";
  description = ''
    The local provider implements resources that operate on the local system.

    They are atypical, as most resources represent a single real world entity that is reached over the network, but a resource like `file` is not singular like that, when NixOps4 is invoked from different environments.
  '';
  sourceBaseUrl = "https://github.com/nixops4/nixops4/tree/main";
  sourceName = "nixops4";
  executable = thisFlake.withSystem resourceProviderSystem (
    { config, ... }: "${config.packages.nixops4-resources-local-release}/bin/nixops4-resources-local"
  );
  resourceTypes = {
    file = {
      description = ''
        Creates or manages a file on the local filesystem.
      '';
      requireState = false;
      inputs = {
        options = {
          name = mkOption {
            type = types.str;
            description = ''
              The path to the file to create or manage.
            '';
          };
          contents = mkOption {
            type = types.str;
            description = ''
              The contents to write to the file.
            '';
          };
        };
      };
      outputs = {
        options = { };
      };
    };
    exec = {
      description = ''
        Executes a command and captures its output.

        It is assumed to be idempotent.
      '';
      requireState = false;
      inputs = {
        options = {
          executable = mkOption {
            type = types.coercedTo (types.functionTo types.str) (withSystem resourceProviderSystem) types.str;
            description = ''
              The command to execute. Can be a string path or a function that returns a path.
            '';
          };
          args = mkOption {
            type = types.listOf (
              types.coercedTo
                (types.oneOf [
                  types.str
                  types.path
                  types.int
                ])
                (x: "${x}")
                types.str
            );
            default = [ ];
            description = ''
              Arguments to pass to the executable.
            '';
          };
          stdin = mkOption {
            type = types.nullOr types.str;
            default = null;
            description = ''
              Optional input to provide on stdin.
            '';
          };
        };
      };
      outputs = {
        options = {
          stdout = mkOption {
            type = types.str;
            description = ''
              The standard output captured from the command.
            '';
          };
        };
      };
    };
    state_file = {
      description = ''
        Provides persistent state storage using a JSON file. This resource implements
        the state provider interface, allowing it to store state for other stateful resources.

        The state file uses [JSON Patch (RFC 6902)](https://tools.ietf.org/rfc/rfc6902.html) operations to track incremental changes,
        enabling efficient updates and providing an audit trail of all state modifications.

        See also: [State](../../state/index.md), [State File Schema](../../schema/state-v0.md)
      '';
      requireState = false;
      inputs = {
        options = {
          name = mkOption {
            type = types.str;
            description = ''
              The path to the JSON state file. This file will store the deployment state
              using a sequence of JSON Patch operations.
            '';
          };
        };
      };
      outputs = {
        options = { };
      };
    };
    memo = {
      description = ''
        A stateful resource that stores an immutable value. Once created, the memo
        retains its initial value regardless of changes to the [`initialize_with` input](#inputs.initialize_with).

        This is useful for preserving configuration values that should remain constant
        throughout a deployment's lifetime, such as the [NixOS `system.stateVersion`](https://search.nixos.org/options?show=system.stateVersion&query=system.stateVersion), or
        perhaps an initial database schema version (in case you need it for conditional
        hotfixes later), or other deployment-specific constants that should not change after
        initial creation.
      '';
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
