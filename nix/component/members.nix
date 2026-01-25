# Defines the `members` option and composite output handling for nested components.
parentArgs@{
  config,
  lib,
  options,
  resourceProviderSystem,
  ...
}:
let
  inherit (lib) mkOption types;
in
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
          ./base-modules.nix
        ];
      }
    );
    default = { };
    description = ''
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
    visible = "shallow";
  };

  config = {
    # Expose members as module arg so users can access sibling outputs: members.X.outputs.Y
    _module.args.members = config.members;

    # Composites need freeformType on outputs to accept children's output values
    # lazyAttrsOf raw avoids check/thunk wrapping at each nesting layer
    outputs =
      { ... }:
      {
        freeformType = lib.mkIf (!options.resourceType.isDefined) (types.lazyAttrsOf types.raw);
      };
  };
}
