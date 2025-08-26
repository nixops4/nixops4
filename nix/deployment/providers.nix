{
  config,
  lib,
  resourceProviderSystem,
  ...
}:
let
  inherit (lib) mkIf mkOption types;

  /**
    providers-specific behavior in `resources`
  */
  resourceModuleExtension =
    { config, options, ... }:
    {
      _class = "nixops4Resource";
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
        requireState = config.type.requireState;
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
    };

in
{
  options = {
    # this option merges with the one in `resources.nix`
    resources = mkOption {
      type = types.lazyAttrsOf (
        types.submoduleWith {
          class = "nixops4Resource";
          modules = [ resourceModuleExtension ];
        }
      );
    };
    providers = mkOption {
      description = ''
        The resource providers to use.

        Resource providers are the executables that implement the operations on resources.

        While provider information can be provided directly in the resource, this indirection allows for the same provider to be used for multiple resources conveniently.

        It also allows for expressions to extract just the providers from a deployment configuration.
      '';
      type = types.lazyAttrsOf (
        types.submoduleWith {
          modules = [ ../provider/provider.nix ];
          class = "nixops4Provider";
          specialArgs = {
            inherit resourceProviderSystem;
          };
        }
      );
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
    _module.args.providers = lib.mapAttrs (
      name: provider:
      lib.mapAttrs (
        k: v:
        # Set the _type, so that an accidental use in `imports` gets caught
        # and reported in a comprehensible way.
        v
        // {
          /**
            A NixOps4 Resource Type can be used in the resource `type` option.
          */
          _type = "nixops4ResourceType";
        }
      ) provider.resourceTypes
    ) config.providers;
  };
}
