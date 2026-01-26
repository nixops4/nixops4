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
            nixops4 = {
              _type = "nixops4Component";
              rootFunction = { outputValues, resourceProviderSystem, ... }: {
                members = {
                  member-one = throw "not implemented one";
                  member-two = throw "not implemented two";
                };
              };
            };
          };
        }
      ''} ./flake.nix
      nixops4 members list > members
      cat 1>&2 members
      grep member-one members
      grep member-two members
      [[ $(wc -l <members) == 2 ]]
      rm members
      rm flake.nix
    ''}");
  '';
}
