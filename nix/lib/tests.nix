/**
  nix-unit tests for `nixops4.lib`
  nix-unit --flake .#tests
 */
{ lib, self, system }:
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
          ({ characteristic, foo, options, ... }:
            assert characteristic == "I'm a special snowflake";

            {
              resources.a =
                # Can't assert this much higher up because _module must be
                # evaluatable before we ask for `foo`, which comes from
                # `_module.args`.
                assert foo == "bar";
                # Similarly:
                assert options._module.args.loc == [ "optionP" "ath" "_module" "args" ];
                { };
            })
        ];
        specialArgs = {
          characteristic = "I'm a special snowflake";
        };
        prefix = [ "optionP" "ath" ];
      };
    in
    {
      "test type" = {
        expr = d._type;
        expected = "nixops4Deployment";
      };
      "test resource set" = {
        expr = d.deploymentFunction { resources = { }; };
        expected = { resources = { a = { }; }; };
      };
    };
}
