# Component export - wraps resource data or exposes nested members.
{
  config,
  lib,
  options,
  ...
}:
let
  inherit (lib) mkOption types;
in
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
}
