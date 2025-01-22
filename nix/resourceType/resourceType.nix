/**
  This type has a lot in common with the `resource.nix` module, but it only
  contains static "metadata", which is a significant difference
*/
{ config, lib, provider, ... }:
let
  inherit (lib) mkOption types;
in
{
  _class = "nixops4ResourceType";
  options = {
    provider.executable = mkOption {
      type = types.str;
      default = provider.executable;
      defaultText = lib.literalMD ''
        inherited from provider
      '';
      description = ''
        Value to be used for [`resources.<name>.provider.executable`](#opt-nixops4Deployments._name_.providers._name_.executable).
      '';
    };

    provider.args = mkOption {
      type = types.listOf types.str;
      default = provider.args;
      defaultText = lib.literalMD ''
        inherited from provider
      '';
      description = ''
        Value to be used for [`resources.<name>.provider.args`](#opt-nixops4Deployments._name_.providers._name_.args).
      '';
    };

    provider.type = mkOption {
      type = types.str;
      default = provider.type;
      defaultText = lib.literalMD ''
        inherited from provider
      '';
      description = ''
        Value to be used for [`resources.<name>.provider.type`](#opt-nixops4Deployments._name_.providers._name_.type).
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
    outputsSkeleton =
      lib.mapAttrs
        (name: opt: { })
        (lib.removeAttrs
          (lib.evalModules {
            modules = [ config.outputs ];
          }).options
          [ "_module" ]
        );
  };
}
