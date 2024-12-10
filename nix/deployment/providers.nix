{ config, lib, resourceProviderSystem, ... }:
let
  inherit (lib) mkIf mkOption types;

  resourceProvider = provider@{ ... }: {
    options = {
      executable = mkOption {
        description = ''
          The path to the executable that implements the resource operations.
        '';
        type = types.str;
      };

      args = mkOption {
        description = ''
          Any command line arguments to pass to the executable.
        '';
        type = types.listOf types.str;
        default = [ ];
      };

      type = mkOption {
        description = ''
          The type of communication to use with the resource provider executable.
        '';
        type = types.str;
        default = "stdio";
      };

      resourceTypes = mkOption {
        description = ''
          The types of resources that this provider can create.

          The purpose of the `resourceTypes` option is to provide the information necessary to create the `providers` module argument.

          The attribute name under `resourceTypes` is the resource type, and gives rise to `providers.<provider>.<resourceType>`.
        '';
        type = types.lazyAttrsOf (types.submoduleWith {
          specialArgs.provider = provider.config;
          modules = [
            resourceType
            ({ name, ... }: { type = name; })
          ];
        });
      };
    };
  };

  /**
    This type has a lot in common with the `resource.nix` module, but it only
    contains static "metadata", which is a significant difference
  */
  resourceType = { config, provider, ... }: {
    options = {
      provider.executable = mkOption {
        type = types.str;
        default = provider.executable;
        defaultText = lib.literalMD ''
          inherited from provider
        '';
        description = ''
          Value to be used for [`resources.<name>.provider.executable`](#resourcesnameproviderexecutable).
        '';
      };

      provider.args = mkOption {
        type = types.listOf types.str;
        default = provider.args;
        defaultText = lib.literalMD ''
          inherited from provider
        '';
        description = ''
          Value to be used for [`resources.<name>.provider.args`](#resourcesnameproviderargs).
        '';
      };

      provider.type = mkOption {
        type = types.str;
        default = provider.type;
        defaultText = lib.literalMD ''
          inherited from provider
        '';
        description = ''
          Value to be used for [`resources.<name>.provider.type`](#resourcesnameprovidertype).
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
  };

  /** providers-specific behavior in `resources` */
  resourceModuleExtension = { config, options, ... }: {
    options.type = mkOption {
      description = ''
        A resource type from the `providers` module argument.
      '';
      type = types.raw;
      example = lib.literalExpression ''
        providers.local.file
      '';
    };
    config = mkIf options.type.isDefined {
      provider.executable = config.type.provider.executable;
      provider.args = config.type.provider.args;
      resourceType = config.type.type;
      outputsSkeleton = config.type.outputsSkeleton;
      inputs = { ... }: { imports = [ config.type.inputs ]; };
      outputs = { ... }: { imports = [ config.type.outputs ]; };
    };
  };

in
{
  options = {
    # this option merges with the one in `resources.nix`
    resources = mkOption {
      type = types.lazyAttrsOf (types.submoduleWith {
        modules = [ resourceModuleExtension ];
      });
    };
    providers = mkOption {
      description = ''
        The resource providers to use.

        Resource providers are the executables that implement the operations on resources.

        While provider information can be provided directly in the resource, this indirection allows for the same provider to be used for multiple resources conveniently.

        It also allows for expressions to extract just the providers from a deployment configuration.
      '';
      type =
        types.lazyAttrsOf
          (types.submoduleWith {
            modules = [ resourceProvider ];
            class = "nixops4Provider";
            specialArgs = {
              inherit resourceProviderSystem;
            };
          });
      default = { };
      example = lib.literalExpression ''
        {
          local = inputs.nixops4.modules.nixops4Provider.local;
          foo = inputs.nixops4-resources-foo.modules.nixops4Provider.default;
        }
      '';
    };
  };
  config = {
    _module.args.providers = lib.mapAttrs
      (name: provider:
        provider.resourceTypes
      )
      config.providers;
  };
}
