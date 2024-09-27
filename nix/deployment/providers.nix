{ config, lib, resourceProviderSystem, ... }:
let
  inherit (lib) mkOption types;

  resourceProvider = provider@{ ... }: {
    options = {
      command = mkOption {
        type = types.str;
      };

      args = mkOption {
        type = types.listOf types.str;
        default = [ ];
      };

      type = mkOption {
        type = types.str;
        default = "stdio";
      };

      resourceTypes = mkOption {
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

  resourceType = { config, provider, ... }: {
    options = {
      provider.command = mkOption {
        type = types.str;
        default = provider.command;
        defaultText = lib.literalMD ''
          inherited from provider
        '';
      };

      provider.args = mkOption {
        type = types.listOf types.str;
        default = provider.args;
        defaultText = lib.literalMD ''
          inherited from provider
        '';
      };

      provider.type = mkOption {
        type = types.str;
        default = provider.type;
        defaultText = lib.literalMD ''
          inherited from provider
        '';
      };

      provider.types = mkOption {
        type = types.raw;
        defaultText = lib.literalMD ''
          derived from `outputs`
        '';
      };

      inputs = mkOption {
        type = types.deferredModule;
      };

      outputs = mkOption {
        type = types.deferredModule;
      };

      type = mkOption {
        type = types.str;
      };
    };
    config = {
      provider.types.${config.type}.outputs =
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

in
{
  options = {
    providers = mkOption {
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
