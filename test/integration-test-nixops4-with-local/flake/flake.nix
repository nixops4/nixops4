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

          # This is a comprehensive nested deployment test
          # It demonstrates resources with cross-deployment dependencies
          # and complex state management across multiple deployment levels
          nestedDeployment =
            {
              lib,
              config,
              providers,
              withResourceProviderSystem,
              resources,
              deployments,
              ...
            }:
            {
              # Parent deployment creates a state file and some config values
              providers.local = inputs.nixops4.modules.nixops4Provider.local;

              resources.parentState = {
                type = providers.local.state_file;
                inputs.name = "nested-parent-state.json";
              };

              resources.parentVersion = {
                type = providers.local.memo;
                state = [ "parentState" ];
                inputs.initialize_with = "v1.0.0";
              };

              resources.parentConfig = {
                type = providers.local.memo;
                state = [ "parentState" ];
                inputs.initialize_with = "production";
              };

              # Child deployment 1 - frontend
              deployments.frontend = {
                resources.webVersion = {
                  type = providers.local.memo;
                  state = [ "parentState" ];
                  inputs.initialize_with = "frontend-${resources.parentVersion.value}";
                };

                resources.webConfig = {
                  type = providers.local.memo;
                  state = [ "parentState" ];
                  inputs.initialize_with = "web-${resources.parentConfig.value}";
                };

                # Nested deployment within frontend
                deployments.assets = {
                  resources.assetVersion = {
                    type = providers.local.memo;
                    state = [ "parentState" ];
                    inputs.initialize_with = "assets-${deployments.frontend.resources.webVersion.value}";
                  };

                  resources.assetConfig = {
                    type = providers.local.memo;
                    state = [ "parentState" ];
                    inputs.initialize_with = "cdn-config-${deployments.frontend.resources.webConfig.value}";
                  };
                };
              };

              # Child deployment 2 - backend
              deployments.backend = {
                resources.apiVersion = {
                  type = providers.local.memo;
                  state = [ "parentState" ];
                  inputs.initialize_with = "api-${resources.parentVersion.value}";
                };

                resources.apiConfig = {
                  type = providers.local.memo;
                  state = [ "parentState" ];
                  inputs.initialize_with = "backend-${resources.parentConfig.value}-frontend-${deployments.frontend.resources.webVersion.value}";
                };

                # Database deployment nested in backend
                deployments.database = {
                  resources.dbVersion = {
                    type = providers.local.memo;
                    state = [ "parentState" ];
                    inputs.initialize_with = "db-${deployments.backend.resources.apiVersion.value}";
                  };

                  resources.dbConfig = {
                    type = providers.local.memo;
                    state = [ "parentState" ];
                    inputs.initialize_with = "postgres-${deployments.backend.resources.apiConfig.value}";
                  };
                };
              };

              # Parent resource that depends on child deployments
              resources.deploymentSummary = {
                type = providers.local.memo;
                state = [ "parentState" ];
                inputs.initialize_with = lib.concatStringsSep "|" [
                  "parent:${resources.parentVersion.value}"
                  "frontend:${deployments.frontend.resources.webVersion.value}"
                  "backend:${deployments.backend.resources.apiVersion.value}"
                  "assets:${deployments.frontend.deployments.assets.resources.assetVersion.value}"
                  "db:${deployments.backend.deployments.database.resources.dbVersion.value}"
                ];
              };
            };

          # Test case: nested deployment with unreferenced resources.
          # The nested deployment has resources that are NOT referenced by
          # any parent resource input. ListNestedDeployments discovers them.
          # Test: UNREFERENCED NESTED DEPLOYMENT in check.nix
          unreferencedNesting =
            {
              providers,
              ...
            }:
            {
              providers.local = inputs.nixops4.modules.nixops4Provider.local;

              # Parent state file
              resources.stateFile = {
                type = providers.local.state_file;
                inputs.name = "unreferenced-nesting-state.json";
              };

              # A simple parent resource - does NOT reference nested deployment
              resources.parentResource = {
                type = providers.local.memo;
                state = [ "stateFile" ];
                inputs.initialize_with = "parent-value";
              };

              # Nested deployment with resources that should be applied
              # but are never referenced by the parent
              deployments.orphan = {
                resources.orphanedResource = {
                  type = providers.local.memo;
                  state = [ "stateFile" ];
                  inputs.initialize_with = "orphan-value";
                };
              };
            };
        };
      }
    );
}
