/**
  A reification of the resource interface between the language and nixops4.
 */
{ config, lib, options, ... }:
let
  inherit (lib) mkOption types;
in
{
  options = {
    _resourceForNixOps = mkOption {
      type = types.lazyAttrsOf types.raw;
      description = ''
        The value that is passed to NixOps to represent this resource.
      '';
      internal = true;
      readOnly = true;
    };
    provider.type = mkOption {
      type = types.enum [ "stdio" ];
      default = "stdio";
      description = ''
        The type of communication to use with the resource provider executable.
      '';
    };
    provider.executable = mkOption {
      type = types.str;
      description = ''
        The path to the executable that implements the resource operations.
      '';
    };
    provider.args = mkOption {
      type = types.listOf (types.coercedTo (types.oneOf [ types.str types.path types.int ]) (x: "${x}") types.str);
      default = [ ];
      description = ''
        Any command line arguments to pass to the executable.
      '';
    };
    resourceType = mkOption {
      type = types.str;
      description = ''
        The type of resource to create. Most resource providers will have some fixed set of resource types.
      '';
    };
    inputs = mkOption {
      type = types.submodule { };
      description = ''
        The inputs to the resource.

        These parameters primarily control the configuration of the resource.
        They are set by you (a module author or configuration author) and are passed to the resource provider executable.
      '';
    };
    outputs = mkOption {
      type = types.submodule { };
      description = ''
        The outputs of the resource.

        These follow from the real world existence of the resource.
        They are set by NixOps, which in turn gets this information from resource providers.
      '';
    };
    outputsSkeleton = mkOption {
      type = types.lazyAttrsOf (types.submodule { });
      description = ''
        The attribute names which occur in the output of the resource.

        This is currently required to be provided "statically" by the Nix expressions.
      '';
      internal = true;
    };
    requireState = mkOption {
      type = types.bool;
      description = ''
        Whether the resource requires state to be stored.
      '';
    };
    state = mkOption {
      type = types.nullOr types.str;
      description = ''
        The state handler for the resource, if needed.
      '';
      default = null;
      apply = x: lib.throwIf (config.requireState && x == null) "${options.state} ${if options.state.highestPrio >= lib.modules.defaultOverridePriority then "has not been defined" else "must not be null"}" x;
    };
  };
  config = {
    _resourceForNixOps = {
      inherit (config) provider inputs outputsSkeleton state;
      type = config.resourceType;
    };
  };
}
