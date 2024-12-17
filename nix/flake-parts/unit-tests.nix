{ nixops4, flake-parts }:
let
  exampleSystem = "aarch64-linux";
in
{
  "empty invocation" =
    let
      flake = flake-parts.lib.mkFlake { inputs = { self = flake /* not super accurate */; }; } {
        systems = [ exampleSystem ];
        imports = [ nixops4.modules.flake.default ];
      };
    in
    {
      "test: check nixops4Deployments" = {
        expr = flake.nixops4Deployments;
        expected = { };
      };
      "test: check checks" = {
        expr = flake.checks;
        expected = {
          "${exampleSystem}" = { };
        };
      };
    };
  "example" =
    let
      flake = flake-parts.lib.mkFlake { inputs = { self = flake; }; } {
        systems = [ exampleSystem ];
        imports = [ nixops4.modules.flake.default ];
        nixops4Deployments.hello = {
          providers.dummy = {
            executable = "/bin/false";
            type = "stdio";
            resourceTypes = throw "not implemented for test";
          };
        };
      };
    in
    {
      "test: check nixops4Deployments" = {
        expr = flake.nixops4Deployments?hello;
        expected = true;
      };
      "test: check checks" = {
        expr =
          flake.checks.${exampleSystem}.nixops-deployment-providers-hello.type;
        expected = "derivation";
      };
      "test: instantiate check" = {
        expr =
          builtins.seq
            flake.checks.${exampleSystem}.nixops-deployment-providers-hello.outPath
            true;
        expected = true;
      };
    };
}
