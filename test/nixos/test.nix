{ config, ... }:
{
  name = "nixops4";

  nodes.deployer =
    { pkgs, ... }:
    {
      users.users.alice = {
        isNormalUser = true;
      };
    };

  testScript = ''
    deployer.succeed("su - alice -c 'nixops4 --version'");

    deployer.succeed("su - alice -c ${config.node.pkgs.writeScript "example" ''
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
      rm -f members
      rm -f flake.nix
    ''}");
  '';
}
