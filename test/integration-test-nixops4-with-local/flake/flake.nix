{
  inputs = {
    nixpkgs.url = "@nixpkgs@";
    nixops4.url = "@nixops4@";
    flake-parts.url = "@flake-parts@";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";
  };

  outputs = inputs@{ flake-parts, ... }: flake-parts.lib.mkFlake { inherit inputs; } (
    { ... }: {
      imports = [
        inputs.nixops4.modules.flake.default
      ];
      systems = [ "@system@" ];
      nixops4Deployments = {
        myDeployment = { providers, withResourceProviderSystem, resources, ... }: {
          providers.local = inputs.nixops4.modules.nixops4Provider.local;
          resources.hello = {
            type = providers.local.exec;
            inputs = {
              # TODO: test framework to run pre-evaluate something like this
              # executable = withResourceProviderSystem ({ pkgs, lib, ... }:
              #   lib.getExe pkgs.hello
              # );
              #
              # Use PATH for now
              executable = "hello";
              args = [ "--greeting" "Hallo wereld" ];
            };
          };
          resources."file.txt" = {
            type = providers.local.file;
            inputs = {
              name = "file.txt";
              contents = resources.hello.stdout;
            };
          };
        };

        failingDeployment = { providers, withResourceProviderSystem, resources, ... }: {
          providers.local = inputs.nixops4.modules.nixops4Provider.local;
          resources.hello = {
            type = providers.local.exec;
            inputs = {
              executable = "die";
              args = [ "oh no, this and that failed" ];
            };
          };
          resources."file.txt" = {
            type = providers.local.file;
            inputs = {
              name = "file.txt";
              contents = resources.hello.stdout;
            };
          };
        };
      };
    }
  );
}
