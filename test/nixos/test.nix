{ config, ... }:
{
  name = "nixops4";

  nodes.deployer =
    { pkgs, ... }:
    {
      # Uncomment if needed
      # environment.systemPackages = [
      #   pkgs.git
      # ];
    };

  testScript = ''
    deployer.succeed("nixops4 --version");

    deployer.succeed("${config.node.pkgs.writeScript "example" ''
      #!${config.node.pkgs.runtimeShell}
      set -euxo pipefail
      mkdir example
      cd example
      cp ${builtins.toFile "flake.nix" ''
        {
          outputs = { ... }: {
            nixops4Deployments = {
              deployment-one = throw "not implemented one";
              deployment-two = throw "not implemented two";
            };
          };
        }
      ''} ./flake.nix
      nixops4 deployments list > deployments
      cat 1>&2 deployments
      grep deployment-one deployments
      grep deployment-two deployments
      [[ $(wc -l <deployments) == 2 ]]
      rm deployments
      rm flake.nix
    ''}");
  '';
}
