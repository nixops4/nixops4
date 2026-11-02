/**
  A reification of the resource interface between the language and nixops4.
*/
{
  config,
  lib,
  options,
  ...
}:
let
  inherit (lib) mkOption types;

  # Option type for referencing a resource, e.g. `members.foo`
  resource = lib.mkOptionType {
    name = "resource";
    description = "resource reference";
    descriptionClass = "noun";
    check = value: value ? absoluteMemberPath;
    merge = lib.mergeUniqueOption { message = "A resource reference cannot be merged."; };
  };

  # Backwards-compatible type that accepts either a resource or a path list
  legacyResource = types.coercedTo (types.listOf types.str) (path: {
    absoluteMemberPath = path;
  }) resource;
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
      type = types.listOf (
        types.coercedTo (types.oneOf [
          types.str
          types.path
          types.int
        ]) (x: "${x}") types.str
      );
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
      type = types.submodule (
        { options, ... }:
        {
          # Ugly smuggle until we can confidently "pretty smuggle" with https://github.com/NixOS/nixpkgs/pull/391544
          options._options = lib.mkOption {
            internal = true;
          };
          config._options = options;
        }
      );
      description = ''
        The inputs to the resource.

        These parameters primarily control the configuration of the resource.
        They are set by you (a module author or configuration author) and are passed to the resource provider executable.
      '';
    };
    isOptionalInputName = mkOption {
      type = types.functionTo types.bool;
      default = _: false;
      description = ''
        Whether the named input is optional or not. If optional and undefined, no error is raised, and it is not passed to the resource provider.
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
      type = types.nullOr legacyResource;
      description = ''
        The state handler for the resource, if needed.
      '';
      default = null;
      apply =
        x:
        lib.warnIf (lib.any lib.isList options.state.definitions)
          "Option ${options.state} is defined as a list. Use a `members.<name>` reference instead."
          lib.throwIf
          (config.requireState && x == null)
          "${options.state} ${
            if options.state.highestPrio >= lib.modules.defaultOverridePriority then
              "has not been defined"
            else
              "must not be null"
          }"
          x;
    };
    absoluteMemberPath = mkOption {
      type = types.listOf types.str;
      description = ''
        The absolute path to this resource from the root component.
      '';
      internal = true;
      readOnly = true;
    };
  };
  config = {
    _resourceForNixOps = {
      inherit (config)
        provider
        outputsSkeleton
        ;
      state = if config.state == null then null else config.state.absoluteMemberPath;

      inputs =
        lib.mapAttrs
          (
            name: value:
            # Provide appropriate values: actual value if defined, null if optional and undefined
            if config.inputs._options.${name}.isDefined then
              value
            else if config.isOptionalInputName name then
              null
            else
              value # Required inputs should always be defined
          )
          (
            lib.filterAttrs (
              name: _v:
              # Only exclude the options retrieval hack
              name != "_options"
            ) config.inputs
          );
      type = config.resourceType;
    };
  };
}
