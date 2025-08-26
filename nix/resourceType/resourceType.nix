/**
  This type has a lot in common with the `resource.nix` module, but it only
  contains static "metadata", which is a significant difference
*/
{
  config,
  lib,
  provider,
  options,
  ...
}:
let
  inherit (lib)
    mkOption
    replaceStrings
    showOption
    types
    ;

  # Polyfill https://github.com/NixOS/nixpkgs/pull/370558
  dropEnd = lib.dropEnd or (n: xs: lib.lists.take (lib.max 0 (lib.lists.length xs - n)) xs);

  moduleLoc = dropEnd 2 options.provider.executable.loc;

  docResources =
    dropEnd
      # usually mounted on providers.<name>.resourceTypes.<name>
      4
      moduleLoc
    ++ [ "resources" ];

  # Incomplete, but good enough for now
  renderFragment = loc: replaceStrings [ "<" ">" ] [ "_" "_" ] (showOption loc);

  linkOptionLoc = loc: "[`" + showOption loc + "`](#opt-" + renderFragment loc + ")";
in
{
  _class = "nixops4ResourceType";
  options = {
    description = mkOption {
      type = types.str;
      description = ''
        A description of what this resource type represents.
      '';
    };

    provider.executable = mkOption {
      type = types.str;
      default = provider.executable;
      defaultText = lib.literalMD ''
        inherited from provider
      '';
      description = ''
        Value to be used for ${
          linkOptionLoc (
            docResources
            ++ [
              "<name>"
              "provider"
              "executable"
            ]
          )
        }.
      '';
    };

    provider.args = mkOption {
      type = types.listOf types.str;
      default = provider.args;
      defaultText = lib.literalMD ''
        inherited from provider
      '';
      description = ''
        Value to be used for ${
          linkOptionLoc (
            docResources
            ++ [
              "<name>"
              "provider"
              "args"
            ]
          )
        }.
      '';
    };

    provider.type = mkOption {
      type = types.str;
      default = provider.type;
      defaultText = lib.literalMD ''
        inherited from provider
      '';
      description = ''
        Value to be used for ${
          linkOptionLoc (
            docResources
            ++ [
              "<name>"
              "provider"
              "type"
            ]
          )
        }.
      '';
    };

    provider.types = mkOption {
      type = types.raw;
      defaultText = lib.literalMD ''
        derived from `outputs`
      '';
      internal = true;
      description = ''
        The output attribute names in a form that nixops4 likes to consume.
        This is a very internal interface, which plays a role in `nixops4` knowing what the output attributes will be before running the provider. This improves laziness and therefore performance and robustness.
      '';
    };

    inputs = mkOption {
      type = types.deferredModule;
      description = ''
        A module that declares the inputs to the resource using its options.
      '';
    };

    outputs = mkOption {
      type = types.deferredModule;
      description = ''
        A module that declares the outputs of the resource using its options.
      '';
    };

    outputsSkeleton = mkOption {
      type = types.attrsOf (types.submodule { });
      internal = true;
      description = ''
        The skeleton of the outputs of the resource - just attribute names.
      '';
    };

    requireState = mkOption {
      type = types.bool;
      description = ''
        Whether the resource requires state to be stored.
      '';
    };

    type = mkOption {
      type = types.str;
      description = ''
        The type of resource to create. Most resource providers will have some fixed set of resource types.
        This selects one of them.

        We suggest to set (override) this only if absolutely necessary for compatibility with earlier versions of a resource.
      '';
      defaultText = lib.literalMD ''
        inherited attribute name
      '';
    };
  };
  config = {
    outputsSkeleton = lib.mapAttrs (name: opt: { }) (
      lib.removeAttrs
        (lib.evalModules {
          modules = [ config.outputs ];
        }).options
        [ "_module" ]
    );
  };
}
