# Polyfill for https://github.com/NixOS/nixpkgs/pull/344407
{ lib, options, pkgs, ... }:
let
  alreadyHasApply = options?system.apply.enable;
in
{
  config = {
    system.activatableSystemBuilderCommands = lib.mkIf (!alreadyHasApply) (lib.mkAfter ''
      mkdir -p $out/bin
      substitute ${./apply.sh} $out/bin/apply \
        --subst-var-by bash ${lib.getExe pkgs.bash} \
        --subst-var-by toplevel ''${!toplevelVar}
      chmod +x $out/bin/apply
    '');
  };
}
