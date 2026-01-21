{
  config,
  lib,
  ...
}:
let
  inherit (lib) mkOption types;
in
{
  options = {
    deployments = mkOption {
      type = types.lazyAttrsOf (
        types.submoduleWith {
          class = "nixops4Deployment";
          modules = [
            ./base-modules.nix
          ];
        }
      );
      default = { };
      description = ''
        Nested deployments within this deployment.

        This allows for
        - hierarchical organization
        - deployments whose *set of* resources depends on its parent's resource outputs
          - conditional resources
          - resource per list item

        Example:
        ```nix
        deployments.frontend = {
          resources.webserver = { ... };
          deployments.api = {
            resources.server = { ... };
            resources.database = { ... };
          };
        };
        ```
      '';
      visible = "shallow";
    };
  };
  config = {
    _export = {
      deployments = lib.mapAttrs (_: res: res._export) config.deployments;
    };
  };
}
