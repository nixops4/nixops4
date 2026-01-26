/**
  nix-unit tests for `nixops4.lib`
  nix-unit --flake .#tests
*/
{
  lib,
  self,
  system,
}:
let
  inherit (lib) mkOption types;
in

{
  "minimal mkRoot call" =
    let
      d = self.lib.mkRoot { modules = [ ]; };
    in
    {
      "test type" = {
        expr = d._type;
        expected = "nixops4Component";
      };
      "test empty members" = {
        expr = d.rootFunction {
          resourceProviderSystem = system;
          outputValues = { };
        };
        expected = {
          members = { };
        };
      };
    };
  "elaborate mkRoot call" =
    let
      localProvider = {
        executable = "/fake/store/asdf-nixops4-resources-local/bin/nixops4-resources-local";
        args = [
          "positively"
          "an argument"
        ];
        type = "stdio";
        resourceTypes = {
          exec = {
            inputs = {
              options.executable = mkOption {
                type = types.str;
              };
            };
            outputs = {
              options.stdout = mkOption {
                type = types.str;
              };
            };
            requireState = false;
          };
          b = {
            type = "bee";
            requireState = false;
          };
        };
      };

      d = self.lib.mkRoot {
        modules = [
          { _module.args.foo = "bar"; }
          { _class = "nixops4Component"; }
          (
            {
              characteristic,
              config,
              foo,
              members,
              options,
              providers,
              ...
            }:
            assert characteristic == "I'm a special snowflake";

            {
              _file = "<elaborate mkRoot call>";
              members.a =
                # Can't assert this much higher up because _module must be
                # evaluatable before we ask for `foo`, which comes from
                # `_module.args`.
                assert foo == "bar";
                # Similarly:
                assert
                  options._module.args.loc == [
                    "optionP"
                    "ath"
                    "_module"
                    "args"
                  ];
                {
                  resourceType = "aye";
                  provider = {
                    executable = "/foo/bin/agree";
                    args = [ "positive" ];
                    type = "stdio";
                  };
                  inputs = { };
                  outputs =
                    { ... }:
                    {
                      options.aResult = mkOption {
                        type = types.str;
                      };
                    };
                  outputsSkeleton.aResult = { };
                  requireState = false;
                };

              members.b = {
                resourceType = "bee";
                provider = {
                  executable = "/foo/bin/bee";
                  args = [ "buzz" ];
                  type = "stdio";
                };
                inputs =
                  { ... }:
                  {
                    options.a = mkOption {
                      type = types.str;
                    };
                    options.a2 = mkOption {
                      type = types.str;
                    };
                    # Access outputs via members.X.outputs.Y
                    config.a = members.a.outputs.aResult;
                    # Or equivalently via config.members.X.outputs.Y
                    config.a2 = config.members.a.outputs.aResult;
                  };
                outputs = { };
                outputsSkeleton = { };
                requireState = false;
              };
              providers.local = localProvider;
              members.install-mgmttool = {
                type = providers.local.exec;
                inputs.executable = "/fake/store/c00lg4l-mgmttool/bin/install-mgmttool";
              };
            }
          )
        ];
        specialArgs = {
          characteristic = "I'm a special snowflake";
        };
        prefix = [
          "optionP"
          "ath"
        ];
      };

      # What NixOps does is effectively:
      #   fix (rootFunction . realWorld)
      #   (or equivalently: fix (realWorld . rootFunction))
      # In this model,
      # forNixOps: intermediate value going from rootFunction to realWorld
      # forExpr: intermediate value going from realWorld to rootFunction
      forNixOps = d.rootFunction forExpr;
      forExpr = {
        resourceProviderSystem = system;
        outputValues = {
          a.aResult = "aye it's a result";
          b = { };
          install-mgmttool = {
            stdout = "mgmttool installing\nmgmttool installed";
          };
        };
      };

    in
    {
      "test type" = {
        expr = d._type;
        expected = "nixops4Component";
      };
      "test members passed to NixOps" = {
        expr = forNixOps;
        expected = {
          members = {
            a = {
              resource = {
                inputs = { };
                provider = {
                  args = [ "positive" ];
                  executable = "/foo/bin/agree";
                  type = "stdio";
                };
                type = "aye";
                outputsSkeleton = {
                  aResult = { };
                };
                state = null;
              };
            };
            b = {
              resource = {
                inputs = {
                  a = "aye it's a result";
                  a2 = "aye it's a result";
                };
                provider = {
                  args = [ "buzz" ];
                  executable = "/foo/bin/bee";
                  type = "stdio";
                };
                type = "bee";
                outputsSkeleton = { };
                state = null;
              };
            };
            install-mgmttool = {
              resource = {
                inputs = {
                  executable = "/fake/store/c00lg4l-mgmttool/bin/install-mgmttool";
                };
                provider = {
                  args = [
                    "positively"
                    "an argument"
                  ];
                  executable = "/fake/store/asdf-nixops4-resources-local/bin/nixops4-resources-local";
                  type = "stdio";
                };
                type = "exec";
                outputsSkeleton = {
                  stdout = { };
                };
                state = null;
              };
            };
          };
        };
      };
    };

  "requireState validation" =
    let
      # Define a test provider module that includes both stateful and stateless resource types
      testProviderModule = {
        executable = "/fake/store/example-provider/bin/example-provider";
        type = "stdio";
        resourceTypes = {
          stateful = {
            requireState = true;
            inputs = {
              options.data = mkOption {
                type = types.str;
              };
            };
            outputs = {
              options = { };
            };
            outputsSkeleton = { };
          };
          stateless = {
            requireState = false;
            inputs = {
              options.data = mkOption {
                type = types.str;
              };
            };
            outputs = {
              options = { };
            };
            outputsSkeleton = { };
          };
        };
      };

      # This should work - stateless resource without state
      validStateless = self.lib.mkRoot {
        modules = [
          (
            { config, providers, ... }:
            {
              providers.example = testProviderModule;
              members.myStateless = {
                type = providers.example.stateless;
                inputs.data = "hello";
              };
            }
          )
        ];
      };

      # This should work - stateful resource with state
      validStateful = self.lib.mkRoot {
        modules = [
          (
            { config, providers, ... }:
            {
              providers.example = testProviderModule;
              members.myStateful = {
                type = providers.example.stateful;
                state = [ "myStateHandler" ];
                inputs.data = "world";
              };
            }
          )
        ];
      };

      # This should fail - stateful resource without state
      invalidStateful = self.lib.mkRoot {
        modules = [
          (
            { config, providers, ... }:
            {
              providers.example = testProviderModule;
              members.myStateful = {
                type = providers.example.stateful;
                inputs.data = "fail";
                # state is missing - this should cause an error due to requireState = true
              };
            }
          )
        ];
      };
    in
    {
      "test stateless resource works without state" = {
        expr =
          (validStateless.rootFunction {
            resourceProviderSystem = system;
            outputValues = {
              myStateless = { };
            };
          }).members.myStateless.resource.state;
        expected = null;
      };

      "test stateful resource works with state" = {
        expr =
          (validStateful.rootFunction {
            resourceProviderSystem = system;
            outputValues = {
              myStateful = { };
            };
          }).members.myStateful.resource.state;
        expected = [ "myStateHandler" ];
      };

      "test stateful resource without state throws error" = {
        expr =
          (invalidStateful.rootFunction {
            resourceProviderSystem = system;
            outputValues = {
              myStateful = { };
            };
          }).members.myStateful.resource.state;
        expectedError.type = "ThrownError";
        expectedError.msg = "members\\.myStateful\\.state has not been defined";
      };
    };

  "nested providers" =
    let
      testProviderModule = {
        executable = "/fake/store/test-provider/bin/test-provider";
        type = "stdio";
        resourceTypes = {
          simple = {
            requireState = false;
            inputs = {
              options.value = mkOption { type = types.str; };
            };
            outputs = {
              options.result = mkOption { type = types.str; };
            };
            outputsSkeleton.result = { };
          };
        };
      };

      # Child component defines its own provider and uses it
      childWithOwnProvider = self.lib.mkRoot {
        modules = [
          {
            members.child =
              { providers, ... }:
              {
                providers.childProvider = testProviderModule;
                type = providers.childProvider.simple;
                inputs.value = "from-child-provider";
              };
          }
        ];
      };

      # Child uses parent's provider (existing behavior)
      childWithParentProvider = self.lib.mkRoot {
        modules = [
          (
            { providers, ... }:
            {
              providers.parentProvider = testProviderModule;
              members.child = {
                type = providers.parentProvider.simple;
                inputs.value = "from-parent-provider";
              };
            }
          )
        ];
      };

    in
    {
      "test child can define and use own provider" = {
        expr =
          (childWithOwnProvider.rootFunction {
            resourceProviderSystem = system;
            outputValues.child = {
              result = "ok";
            };
          }).members.child.resource.inputs.value;
        expected = "from-child-provider";
      };

      "test child can use parent provider" = {
        expr =
          (childWithParentProvider.rootFunction {
            resourceProviderSystem = system;
            outputValues.child = {
              result = "ok";
            };
          }).members.child.resource.inputs.value;
        expected = "from-parent-provider";
      };
    };
}
