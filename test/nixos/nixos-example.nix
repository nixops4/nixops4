{ config, lib, ... }:
let
  inherit (lib) fileset;
in
{
  name = "nixops4";

  nodes.deployer = { pkgs, ... }: {
    environment.systemPackages = [
      pkgs.git
    ];
  };

  nodes.target = { pkgs, ... }: { };

  testScript = ''
    start_all();
    deployer.wait_for_unit("multi-user.target");
    target.wait_for_unit("multi-user.target");

    deployer.succeed("nixops4 --version");

    deployer.succeed("${config.node.pkgs.writeScript "example" ''
      #!${config.node.pkgs.runtimeShell}
      set -euxo pipefail
      mkdir example
      cd example
      # FIXME: copy dot files
      cp -rT --no-preserve=mode ${
        fileset.toSource {
          fileset =
            fileset.unions [
              ../../flake.nix
              ../../dev
            ];
          root = ../..;
        }
      } .
      nixops4 deployments list > deployments
      cat 1>&2 deployments
      grep default deployments
      nixops4 deploy
    ''}");
  '';
}
