/**
  nix-unit tests for `nixops4.lib`
  nix-unit --flake .#tests
 */
{ lib, self, system }:
let
  inherit (lib) mkOption types;
in

{
  "minimal mkDeployment call" =
    let
      d = self.lib.mkDeployment { modules = [ ]; };
    in
    {
      "test type" = {
        expr = d._type;
        expected = "nixops4Deployment";
      };
      "test resource set" = {
        expr = d.deploymentFunction { resources = { }; resourceProviderSystem = system; };
        expected = { resources = { }; };
      };
    };
  "elaborate mkDeployment call" =
    let
      localProvider = {
        executable = "/fake/store/asdf-nixops4-resources-local/bin/nixops4-resources-local";
        args = [ "positively" "an argument" ];
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
          b = { type = "bee"; requireState = false; };
        };
      };

      d = self.lib.mkDeployment {
        modules = [
          { _module.args.foo = "bar"; }
          { _class = "nixops4Deployment"; }
          ({ characteristic, config, foo, options, resources, providers, ... }:
            assert characteristic == "I'm a special snowflake";

            {
              _file = "<elaborate mkDeployment call>";
              resources.a =
                # Can't assert this much higher up because _module must be
                # evaluatable before we ask for `foo`, which comes from
                # `_module.args`.
                assert foo == "bar";
                # Similarly:
                assert options._module.args.loc == [ "optionP" "ath" "_module" "args" ];
                {
                  resourceType = "aye";
                  provider = {
                    executable = "/foo/bin/agree";
                    args = [ "positive" ];
                    type = "stdio";
                  };
                  inputs = { };
                  outputs = { ... }: {
                    options.aResult = mkOption {
                      type = types.str;
                    };
                  };
                  outputsSkeleton.aResult = { };
                  requireState = false;
                };

              resources.b = {
                resourceType = "bee";
                provider = {
                  executable = "/foo/bin/bee";
                  args = [ "buzz" ];
                  type = "stdio";
                };
                inputs = { ... }: {
                  options.a = mkOption {
                    type = types.str;
                  };
                  options.a2 = mkOption {
                    type = types.str;
                  };
                  config.a = resources.a.aResult;
                  config.a2 = config.resources.a.outputs.aResult;
                };
                outputs = { };
                outputsSkeleton = { };
                requireState = false;
              };
              providers.local = localProvider;
              resources.install-mgmttool = {
                type = providers.local.exec;
                inputs.executable = "/fake/store/c00lg4l-mgmttool/bin/install-mgmttool";
              };
            })
        ];
        specialArgs = {
          characteristic = "I'm a special snowflake";
        };
        prefix = [ "optionP" "ath" ];
      };

      # What NixOps does is effectively:
      #   fix (deploymentFunction . realWorld)
      #   (or equivalently: fix (realWorld . deploymentFunction))
      # In this model,
      # forNixOps: intermediate value going from deploymentFunction to realWorld
      # forExpr: intermediate value going from realWorld to deploymentFunction
      forNixOps = d.deploymentFunction forExpr;
      forExpr = {
        resourceProviderSystem = system;
        resources = {
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
        expected = "nixops4Deployment";
      };
      "test resource set passed to NixOps" = {
        expr = forNixOps;
        expected = {
          resources = {
            a = {
              inputs = { };
              provider = {
                args = [ "positive" ];
                executable = "/foo/bin/agree";
                type = "stdio";
              };
              type = "aye";
              outputsSkeleton = { aResult = { }; };
              state = null;
            };
            b = {
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
            install-mgmttool = {
              inputs = {
                executable = "/fake/store/c00lg4l-mgmttool/bin/install-mgmttool";
              };
              provider = {
                args = [ "positively" "an argument" ];
                executable = "/fake/store/asdf-nixops4-resources-local/bin/nixops4-resources-local";
                type = "stdio";
              };
              type = "exec";
              outputsSkeleton = { stdout = { }; };
              state = null;
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
      validStateless = self.lib.mkDeployment {
        modules = [
          ({ config, providers, ... }: {
            providers.example = testProviderModule;
            resources.myStateless = {
              type = providers.example.stateless;
              inputs.data = "hello";
            };
          })
        ];
      };

      # This should work - stateful resource with state
      validStateful = self.lib.mkDeployment {
        modules = [
          ({ config, providers, ... }: {
            providers.example = testProviderModule;
            resources.myStateful = {
              type = providers.example.stateful;
              state = "myStateHandler";
              inputs.data = "world";
            };
          })
        ];
      };

      # This should fail - stateful resource without state
      invalidStateful = self.lib.mkDeployment {
        modules = [
          ({ config, providers, ... }: {
            providers.example = testProviderModule;
            resources.myStateful = {
              type = providers.example.stateful;
              inputs.data = "fail";
              # state is missing - this should cause an error due to requireState = true
            };
          })
        ];
      };
    in
    {
      "test stateless resource works without state" = {
        expr = (validStateless.deploymentFunction { resources = { myStateless = { }; }; resourceProviderSystem = system; }).resources.myStateless.state;
        expected = null;
      };

      "test stateful resource works with state" = {
        expr = (validStateful.deploymentFunction { resources = { myStateful = { }; }; resourceProviderSystem = system; }).resources.myStateful.state;
        expected = "myStateHandler";
      };

      "test stateful resource without state throws error" = {
        expr = (invalidStateful.deploymentFunction { resources = { myStateful = { }; }; resourceProviderSystem = system; }).resources.myStateful.state;
        expectedError.type = "ThrownError";
        expectedError.msg = "resources\\.myStateful\\.state has not been defined";
      };
    };
}
