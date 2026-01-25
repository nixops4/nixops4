{ nixops4, flake-parts }:
let
  exampleSystem = "aarch64-linux";
in
{
  "empty invocation" =
    let
      flake =
        flake-parts.lib.mkFlake
          {
            inputs = {
              self = flake # not super accurate
              ;
            };
          }
          {
            systems = [ exampleSystem ];
            imports = [ nixops4.modules.flake.default ];
          };
    in
    {
      # Empty root always exists (no members, but the root component is present)
      "test: check nixops4" = {
        expr = flake ? nixops4;
        expected = true;
      };
      "test: check checks" = {
        expr = flake.checks.${exampleSystem} ? nixops-providers;
        expected = true;
      };
    };
  "example" =
    let
      flake =
        flake-parts.lib.mkFlake
          {
            inputs = {
              self = flake;
            };
          }
          {
            systems = [ exampleSystem ];
            imports = [ nixops4.modules.flake.default ];
            nixops4 = {
              providers.dummy = {
                executable = "/bin/false";
                type = "stdio";
                resourceTypes = throw "not implemented for test";
              };
            };
          };
    in
    {
      "test: check nixops4" = {
        expr = flake.nixops4 != null;
        expected = true;
      };
      "test: check checks" = {
        expr = flake.checks.${exampleSystem}.nixops-providers.type;
        expected = "derivation";
      };
      "test: instantiate check" = {
        expr = builtins.seq flake.checks.${exampleSystem}.nixops-providers.outPath true;
        expected = true;
      };
    };
}
