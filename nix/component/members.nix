{ lib, ... }:
let
  inherit (lib) mkIf mkOption types;

  # Component export - wraps resource data or exposes nested members
  componentExport =
    { config, options, ... }:
    {
      options._export = mkOption {
        type = types.raw;
        internal = true;
        readOnly = true;
      };
      config = {
        _export =
          # A component is a resource if resourceType is defined (set by provider type or manually)
          # Otherwise it's a composite (has nested members)
          if options.resourceType.isDefined then
            { resource = config._resourceForNixOps; }
          else
            { members = lib.mapAttrs (_: comp: comp._export) config.members; };
      };
    };

  /**
    Provider-specific behavior for component modules.
    Adds the `type` option that configures the resource from a provider type.
  */
  componentModuleExtension =
    { config, options, ... }:
    {
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

  membersDescription = ''
    The member components.

    A member is either:
    - A resource component: has resource options (type, inputs, outputs, etc.)
    - A composite component: has nested members

    Example:
    ```nix
    members.webServer = {
      type = providers.local.exec;
      inputs.executable = "nginx";
    };
    members.database = {
      members.primary = { ... };
      members.replica = { ... };
    };
    ```
  '';

  # Composites need freeformType on outputs to accept children's output values
  compositeOutputs =
    { options, ... }:
    {
      outputs =
        { ... }:
        {
          # lazyAttrsOf raw avoids check/thunk wrapping at each nesting layer
          freeformType = lib.mkIf (!options.resourceType.isDefined) (types.lazyAttrsOf types.raw);
        };
    };

  # Modules for every component
  componentModules = [
    ./resource.nix
    componentExport
    componentModuleExtension
    compositeOutputs
    membersModule
  ];

  # Defines the members option and exposes it as a module arg for user access.
  # Each child's outputs come from parent's config.outputs.${name}.
  membersModule =
    parentArgs@{ config, resourceProviderSystem, ... }:
    {
      options.members = mkOption {
        type = types.lazyAttrsOf (
          types.submoduleWith {
            class = "nixops4Component";
            specialArgs = {
              inherit resourceProviderSystem;
            };
            modules = [
              # Inject outputs from parent's config.outputs.${name}
              (
                { name, ... }:
                {
                  outputs =
                    { ... }:
                    {
                      config = parentArgs.config.outputs.${name} or { };
                    };
                }
              )
            ]
            ++ componentModules;
          }
        );
        default = { };
        description = membersDescription;
        visible = "shallow";
      };
      # Expose members as module arg so users can access sibling outputs: members.X.outputs.Y
      config._module.args.members = config.members;
    };

in
# The deployment IS the root composite component.
# It uses the same modules as nested components, except output injection
# (root's outputs are injected by evalDeployment).
{
  imports = [
    ./resource.nix
    # TODO: support root-as-resource
    componentExport
    componentModuleExtension
    compositeOutputs
    membersModule
  ];
}
