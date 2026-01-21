# Part of test: ../check.nix
# It can only be used as part of that.
{
  inputs = {
    nixpkgs.url = "@nixpkgs@";
    nixops4.url = "@nixops4@";
    flake-parts.url = "@flake-parts@";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } (
      { ... }:
      {
        imports = [
          inputs.nixops4.modules.flake.default
        ];
        systems = [ "@system@" ];
        nixops4Deployments = {
          myDeployment =
            {
              providers,
              withResourceProviderSystem,
              resources,
              ...
            }:
            {
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
                  args = [
                    "--greeting"
                    "Hallo wereld"
                  ];
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

          failingDeployment =
            {
              providers,
              withResourceProviderSystem,
              resources,
              ...
            }:
            {
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

          statefulDeployment =
            {
              lib,
              config,
              providers,
              withResourceProviderSystem,
              resources,
              ...
            }:
            {
              options = {
                currentVersion = lib.mkOption {
                  description = ''
                    This represents the development of this deployment.
                    We could have had source code, references to packages, etc, but we're actually deploying just a version number for simplicity.
                  '';
                };
              };
              config = {
                currentVersion = 1;

                providers.local = inputs.nixops4.modules.nixops4Provider.local;

                resources.state = {
                  type = providers.local.state_file;
                  inputs.name = "nixops4-state.json";
                };

                resources.initial_version = {
                  type = providers.local.memo;
                  state = [ "state" ];
                  inputs.initialize_with = config.currentVersion;
                };

                resources.initial_version_file = {
                  type = providers.local.file;
                  inputs.name = "initial-version.md";
                  inputs.contents = ''
                    # This is a fake deployment, which is aware of which version was initially deployed, much like NixOS stateVersion
                    My initial version: ${toString resources.initial_version.value}
                  '';
                };

                resources.current_version_file = {
                  type = providers.local.file;
                  inputs.name = "current-version.md";
                  inputs.contents = ''
                    We're now at version ${toString config.currentVersion}.
                  '';
                };
              };
            };
        };
      }
    );
}
