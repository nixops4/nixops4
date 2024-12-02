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
      d = self.lib.mkDeployment {
        modules = [
          { _module.args.foo = "bar"; }
          ({ characteristic, config, foo, options, resources, ... }:
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
            };
          };
        };
      };
    };
}
