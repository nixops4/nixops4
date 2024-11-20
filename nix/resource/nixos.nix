thisFlake@{ self, withSystem }:

{ config, lib, resourceProviderSystem, ... }:
let
  inherit (lib) mkOption types;

in
{
  options = {
    nixpkgs = mkOption {
      type =
        # Why wasn't my flake type merged :(
        types.flake or types.raw;
    };
    nixos.module = mkOption {
      type = types.deferredModule;
    };
    nixos.configuration = mkOption {
      type = types.raw;
      readOnly = true;
    };
    nixos.specialArgs = mkOption {
      type = types.raw;
      default = { };
    };
    ssh.user = mkOption {
      type = types.nullOr types.str;
      default = "root";
    };
    ssh.host = mkOption {
      type = types.str;
    };
    ssh.hostPublicKey = mkOption {
      type = types.str;
    };
    ssh.opts = mkOption {
      type = types.str;
      default = "";
    };
  };
  config = {
    nixos = {
      configuration = config.nixpkgs.lib.nixosSystem {
        modules = [
          config.nixos.module
          thisFlake.self.modules.nixos.apply
        ];
        specialArgs = config.nixos.specialArgs;
      };
    };
    inputs = {
      command = "${thisFlake.withSystem resourceProviderSystem ({ pkgs, ... }: lib.getExe pkgs.bash)}";
      args = [
        "-c"
        ''
          set -euo pipefail
          export NIX_SSHOPTS="-o StrictHostKeyChecking=yes -o UserKnownHostsFile=${
            # FIXME: when misconfigured, and this contains a private key, we leak it to the store
            builtins.toFile "known_hosts" ''
              ${config.ssh.host} ${config.ssh.hostPublicKey}
            ''} "${lib.strings.escapeShellArg "${config.ssh.opts}"}
          nix copy --to "ssh-ng://$0" "$1" --no-check-sigs --extra-experimental-features nix-command
          ssh $NIX_SSHOPTS "$0" "$1/bin/apply switch"
        ''
        "${lib.optionalString (config.ssh.user != null) "${config.ssh.user}@"}${config.ssh.host}"
        config.nixos.configuration.config.system.build.toplevel
      ];
    };
  };
}
