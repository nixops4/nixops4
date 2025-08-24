{ lib, flake-parts-lib, ... }:
let
  inherit (lib) mkOption types;
in
{
  options.perSystem = flake-parts-lib.mkPerSystemOption (
    { ... }:
    {
      options.builders = mkOption {
        type = types.lazyAttrsOf (types.functionTo types.raw);
        description = ''
          A library of functions that produce derivations.
        '';
      };
    }
  );
}
