{ lib, ... }:
let
  inherit (lib) mkOption types;
in
{
  options = {
    _export = mkOption {
      type = types.lazyAttrsOf types.raw;
      description = "Raw values for the NixOps program to consume";
      internal = true;
    };
  };
}
