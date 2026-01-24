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
              members,
              ...
            }:
            {
              providers.local = inputs.nixops4.modules.nixops4Provider.local;
              members.hello = {
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
              members."file.txt" = {
                type = providers.local.file;
                inputs = {
                  name = "file.txt";
                  contents = members.hello.outputs.stdout;
                };
              };
            };

          failingDeployment =
            {
              providers,
              withResourceProviderSystem,
              members,
              ...
            }:
            {
              providers.local = inputs.nixops4.modules.nixops4Provider.local;
              members.hello = {
                type = providers.local.exec;
                inputs = {
                  executable = "die";
                  args = [ "oh no, this and that failed" ];
                };
              };
              members."file.txt" = {
                type = providers.local.file;
                inputs = {
                  name = "file.txt";
                  contents = members.hello.outputs.stdout;
                };
              };
            };

          statefulDeployment =
            {
              lib,
              config,
              providers,
              withResourceProviderSystem,
              members,
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

                members.state = {
                  type = providers.local.state_file;
                  inputs.name = "nixops4-state.json";
                };

                members.initial_version = {
                  type = providers.local.memo;
                  state = [ "state" ];
                  inputs.initialize_with = config.currentVersion;
                };

                members.initial_version_file = {
                  type = providers.local.file;
                  inputs.name = "initial-version.md";
                  inputs.contents = ''
                    # This is a fake deployment, which is aware of which version was initially deployed, much like NixOS stateVersion
                    My initial version: ${toString members.initial_version.outputs.value}
                  '';
                };

                members.current_version_file = {
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
              members,
              ...
            }:
            {
              # Parent deployment creates a state file and some config values
              providers.local = inputs.nixops4.modules.nixops4Provider.local;

              members.parentState = {
                type = providers.local.state_file;
                inputs.name = "nested-parent-state.json";
              };

              members.parentVersion = {
                type = providers.local.memo;
                state = [ "parentState" ];
                inputs.initialize_with = "v1.0.0";
              };

              members.parentConfig = {
                type = providers.local.memo;
                state = [ "parentState" ];
                inputs.initialize_with = "production";
              };

              # Child composite 1 - frontend
              members.frontend = {
                members.webVersion = {
                  type = providers.local.memo;
                  state = [ "parentState" ];
                  inputs.initialize_with = "frontend-${members.parentVersion.outputs.value}";
                };

                members.webConfig = {
                  type = providers.local.memo;
                  state = [ "parentState" ];
                  inputs.initialize_with = "web-${members.parentConfig.outputs.value}";
                };

                # Nested composite within frontend
                members.assets = {
                  members.assetVersion = {
                    type = providers.local.memo;
                    state = [ "parentState" ];
                    inputs.initialize_with = "assets-${members.frontend.members.webVersion.outputs.value}";
                  };

                  members.assetConfig = {
                    type = providers.local.memo;
                    state = [ "parentState" ];
                    inputs.initialize_with = "cdn-config-${members.frontend.members.webConfig.outputs.value}";
                  };
                };
              };

              # Child composite 2 - backend
              members.backend = {
                members.apiVersion = {
                  type = providers.local.memo;
                  state = [ "parentState" ];
                  inputs.initialize_with = "api-${members.parentVersion.outputs.value}";
                };

                members.apiConfig = {
                  type = providers.local.memo;
                  state = [ "parentState" ];
                  inputs.initialize_with = "backend-${members.parentConfig.outputs.value}-frontend-${members.frontend.members.webVersion.outputs.value}";
                };

                # Database composite nested in backend
                members.database = {
                  members.dbVersion = {
                    type = providers.local.memo;
                    state = [ "parentState" ];
                    inputs.initialize_with = "db-${members.backend.members.apiVersion.outputs.value}";
                  };

                  members.dbConfig = {
                    type = providers.local.memo;
                    state = [ "parentState" ];
                    inputs.initialize_with = "postgres-${members.backend.members.apiConfig.outputs.value}";
                  };
                };
              };

              # Parent resource that depends on child members
              members.deploymentSummary = {
                type = providers.local.memo;
                state = [ "parentState" ];
                inputs.initialize_with = lib.concatStringsSep "|" [
                  "parent:${members.parentVersion.outputs.value}"
                  "frontend:${members.frontend.members.webVersion.outputs.value}"
                  "backend:${members.backend.members.apiVersion.outputs.value}"
                  "assets:${members.frontend.members.assets.members.assetVersion.outputs.value}"
                  "db:${members.backend.members.database.members.dbVersion.outputs.value}"
                ];
              };
            };

          # Test case: structural dependency on nested composite's members.
          # The SET of members inside a composite depends on a sibling resource output.
          # ListMembers for the nested composite will detect a structural dependency
          # because it needs the resource output to determine which members exist.
          structuralDeploymentsAttr =
            {
              lib,
              providers,
              members,
              ...
            }:
            {
              providers.local = inputs.nixops4.modules.nixops4Provider.local;

              members.stateFile = {
                type = providers.local.state_file;
                inputs.name = "structural-deployments-state.json";
              };

              # This resource's output determines which members exist in the nested composite
              members.selector = {
                type = providers.local.memo;
                state = [ "stateFile" ];
                inputs.initialize_with = "enabled";
              };

              # The composite always exists, but its members are conditional.
              # When enumerating conditionalChild's members, the evaluator detects
              # a structural dependency on selector.outputs.value.
              members.conditionalChild = {
                members = lib.optionalAttrs (members.selector.outputs.value == "enabled") {
                  childResource = {
                    type = providers.local.memo;
                    state = [ "stateFile" ];
                    inputs.initialize_with = "child-value";
                  };
                };
              };
            };

          # Test case: structural dependency on resources within a nested composite.
          # The SET of resources in a nested composite depends on a parent resource output.
          # ListMembers for the nested composite will detect a structural dependency.
          structuralResourcesAttr =
            {
              lib,
              providers,
              members,
              ...
            }:
            {
              providers.local = inputs.nixops4.modules.nixops4Provider.local;

              members.stateFile = {
                type = providers.local.state_file;
                inputs.name = "structural-resources-state.json";
              };

              # This resource's output determines which resources exist in the child
              members.selector = {
                type = providers.local.memo;
                state = [ "stateFile" ];
                inputs.initialize_with = "enabled";
              };

              # The composite always exists, but its members are conditional.
              # This is the same pattern as structuralDeploymentsAttr but tests
              # that it works at any nesting level.
              members.child = {
                members = lib.optionalAttrs (members.selector.outputs.value == "enabled") {
                  conditionalResource = {
                    type = providers.local.memo;
                    state = [ "stateFile" ];
                    inputs.initialize_with = "conditional-value";
                  };
                };
              };
            };

          # Test case: dynamic member kind based on resource output.
          # The member's KIND (resource vs composite) depends on a resource output.
          # LoadMember needs to resolve this dependency to determine the kind.
          # Test: DYNAMIC MEMBER KIND in check.nix
          dynamicKind =
            {
              lib,
              providers,
              members,
              ...
            }:
            {
              providers.local = inputs.nixops4.modules.nixops4Provider.local;

              members.stateFile = {
                type = providers.local.state_file;
                inputs.name = "dynamic-kind-state.json";
              };

              # This resource's output determines whether dynamicMember is a resource or composite
              members.selector = {
                type = providers.local.memo;
                state = [ "stateFile" ];
                inputs.initialize_with = "resource"; # could be "composite" to test the other branch
              };

              # This member's kind depends on selector.outputs.value
              members.dynamicMember =
                if members.selector.outputs.value == "resource" then
                  {
                    type = providers.local.memo;
                    state = [ "stateFile" ];
                    inputs.initialize_with = "I am a resource";
                  }
                else
                  {
                    members.nestedResource = {
                      type = providers.local.memo;
                      state = [ "stateFile" ];
                      inputs.initialize_with = "I am inside a composite";
                    };
                  };
            };

          # Test case: nested composite with unreferenced resources.
          # The nested composite has resources that are NOT referenced by
          # any parent resource input. ListMembers discovers them.
          # Test: UNREFERENCED NESTED COMPOSITE in check.nix
          unreferencedNesting =
            {
              providers,
              ...
            }:
            {
              providers.local = inputs.nixops4.modules.nixops4Provider.local;

              # Parent state file
              members.stateFile = {
                type = providers.local.state_file;
                inputs.name = "unreferenced-nesting-state.json";
              };

              # A simple parent resource - does NOT reference nested composite
              members.parentResource = {
                type = providers.local.memo;
                state = [ "stateFile" ];
                inputs.initialize_with = "parent-value";
              };

              # Nested composite with resources that should be applied
              # but are never referenced by the parent
              members.orphan = {
                members.orphanedResource = {
                  type = providers.local.memo;
                  state = [ "stateFile" ];
                  inputs.initialize_with = "orphan-value";
                };
              };
            };

          # Test case: state reference points to a composite instead of a resource.
          # This should trigger: "Expected resource at {path}, but found composite"
          statePointsToComposite =
            {
              providers,
              ...
            }:
            {
              providers.local = inputs.nixops4.modules.nixops4Provider.local;

              # A nested composite (not a resource)
              members.nestedComposite = {
                members.innerResource = {
                  type = providers.local.exec;
                  inputs.executable = "true";
                  inputs.args = [ ];
                };
              };

              # This resource's state incorrectly points to the composite
              members.badResource = {
                type = providers.local.memo;
                state = [ "nestedComposite" ]; # ERROR: points to composite, not resource!
                inputs.initialize_with = "will-fail";
              };
            };

          # Test case: state reference points to a non-existent member.
          # This should trigger an error during member loading.
          statePointsToNonexistent =
            {
              providers,
              ...
            }:
            {
              providers.local = inputs.nixops4.modules.nixops4Provider.local;

              # This resource's state points to a member that doesn't exist
              members.badResource = {
                type = providers.local.memo;
                state = [ "nonExistentMember" ]; # ERROR: no such member!
                inputs.initialize_with = "will-fail";
              };
            };

          # Test case: state reference points to a resource in a non-existent composite.
          # This should trigger: "Expected composite at {path}, but found resource"
          # or a member loading error.
          stateInNonexistentComposite =
            {
              providers,
              ...
            }:
            {
              providers.local = inputs.nixops4.modules.nixops4Provider.local;

              # A simple resource (not a composite)
              members.simpleResource = {
                type = providers.local.exec;
                inputs.executable = "true";
                inputs.args = [ ];
              };

              # This resource's state tries to access a child of a resource
              # (treating the resource as if it were a composite)
              members.badResource = {
                type = providers.local.memo;
                state = [
                  "simpleResource"
                  "child"
                ]; # ERROR: simpleResource is not a composite!
                inputs.initialize_with = "will-fail";
              };
            };

          # Test case: circular dependency between resources.
          # A's input depends on B's output, B's input depends on A's output.
          # Nix evaluation doesn't infinite loop because outputs are special thunks.
          # TaskTracker detects the cycle when resolving dependencies.
          circularDependency =
            {
              providers,
              members,
              ...
            }:
            {
              providers.local = inputs.nixops4.modules.nixops4Provider.local;

              members.stateFile = {
                type = providers.local.state_file;
                inputs.name = "circular-state.json";
              };

              members.resourceA = {
                type = providers.local.memo;
                state = [ "stateFile" ];
                inputs.initialize_with = members.resourceB.outputs.value;
              };

              members.resourceB = {
                type = providers.local.memo;
                state = [ "stateFile" ];
                inputs.initialize_with = members.resourceA.outputs.value;
              };
            };

          # Test case: cycle involving structural dependency - members is bottom.
          # Without the child wrapper, accessing members.X while defining members
          # via optionalAttrs creates infinite recursion at the Nix level.
          # This tests that we get a proper error (not hang or crash).
          structuralCycle =
            {
              lib,
              providers,
              members,
              ...
            }:
            {
              providers.local = inputs.nixops4.modules.nixops4Provider.local;

              members.stateFile = {
                type = providers.local.state_file;
                inputs.name = "structural-cycle-state.json";
              };

              # selector's input depends on inner's output (sibling)
              members.selector = {
                type = providers.local.memo;
                state = [ "stateFile" ];
                inputs.initialize_with = members.inner.outputs.value;
              };

              # inner always exists
              members.inner = {
                type = providers.local.memo;
                state = [ "stateFile" ];
                inputs.initialize_with = "inner-value";
              };
            }
            # This merge creates a structural dependency: listing members needs selector's output
            # But selector's input needs inner's output, which is also in members
            # Result: infinite recursion when evaluating the merged members
            // {
              members = lib.optionalAttrs (members.selector.outputs.value == "trigger") {
                conditional = {
                  type = providers.local.memo;
                  state = [ "stateFile" ];
                  inputs.initialize_with = "conditional-value";
                };
              };
            };
        };
      }
    );
}
