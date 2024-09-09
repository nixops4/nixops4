{
  description = "Description for the project";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } ({ lib, withSystem, ... }: {
      imports = [
        ./nixops4-flake-module.nix
      ];
      systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" "x86_64-darwin" ];
      nixops4Deployments = {
        default = { lib, providers, config, resourceProviderSystem, ... }:
          let
            # something like inputs.providers-local.nixops4Provider
            local = import ./provider-local.nix { inherit withSystem; };
          in
          {
            providers.local = local;
            resources.hello = {
              type = providers.local.exec;
              inputs = {
                command = { pkgs, ... }: lib.getExe pkgs.hello;
                args = [ "--greeting" "Hallo wereld" ];
              };
            };
            resources.recycled = {
              type = providers.local.exec;
              inputs = {
                command = { pkgs, ... }: lib.getExe pkgs.hello;
                args = [ "--greeting" config.resources.hello.outputs.stdout ];
              };
            };
          };
      };
      perSystem = { config, self', inputs', pkgs, system, ... }: { };
      flake = { };
    });
}
