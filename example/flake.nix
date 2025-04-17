{
  description = "Description for the project";

  inputs = {
    # flake-parts.url = "github:hercules-ci/flake-parts";
    # nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # stack overflow, Nix bug in call-flake.nix?
    flake-parts.follows = "nixops4/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";
    nixpkgs.follows = "nixops4/nixpkgs";
    nixops4.url = ../.;
  };

  outputs = inputs@{ flake-parts, nixops4, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [
        nixops4.modules.flake.default
      ];
      systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" "x86_64-darwin" ];
      perSystem = { config, self', inputs', pkgs, system, ... }: { };
      # nixops4 apply --override-input nixops4 ..
      nixops4Deployments.default = { providers, resources, ... }: {
        providers.local = nixops4.modules.nixops4Provider.local;
        resources.state = {
          type = providers.local.state_file;
          inputs.name = "nixops4-state.json";
        };
        resources.test3 = {
          type = providers.local.memo;
          state = "state";
          inputs.initialize_with = { hello = "world"; };
        };

        resources.demoA = {
          type = providers.local.exec;
          inputs = {
            executable = "sh";
            args = [ "-c" "sleep 1; echo hello ${resources.demoB.stdout}" ];
          };
        };
        resources.demoB = {
          type = providers.local.exec;
          inputs = {
            executable = "sh";
            args = [ "-c" "sleep 1; echo world" ];
          };
        };
        resources.demoC = {
          type = providers.local.exec;
          inputs = {
            executable = "sh";
            args = [ "-c" "sleep 1; echo hello ${resources.demoB.stdout}" ];
          };
        };
        resources.demoD = {
          type = providers.local.exec;
          inputs = {
            executable = "sh";
            args = [ "-c" "sleep 1; echo it says ${resources.demoC.stdout}" ];
          };
        };
        # resource

        # resources.helloA = {
        #   type = providers.local.exec;
        #   inputs.executable = resources.helloB.stdout;
        # };
        # resources.helloB = {
        #   type = providers.local.exec;
        #   inputs.executable = resources.helloA.stdout;
        # };
        # resources.test2 = {
        #   type = builtins.seq resources.test3.value providers.local.memo;
        #   state = "state";
        #   inputs.initialize_with = { hello = "world"; };
        # };
      };
      flake = { };
    };
}
