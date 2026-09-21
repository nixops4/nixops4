thisFlake@{ withSystem }:

# Test-only provider module exposing `nixops4-resources-fixture`.
{
  lib,
  resourceProviderSystem,
  ...
}:
let
  inherit (lib) mkOption types;
in
{
  name = "Fixture Provider";
  description = ''
    Test-only provider exercising the NixOps4 resource-protocol
    surface. Not for real deployments.
  '';
  sourceBaseUrl = "https://github.com/nixops4/nixops4/tree/main";
  sourceName = "nixops4";
  executable = thisFlake.withSystem resourceProviderSystem (
    { config, ... }:
    "${config.packages.nixops4-resources-fixture-release}/bin/nixops4-resources-fixture"
  );
  resourceTypes = {
    stateful_state_file = {
      description = ''
        A state resource whose own state has to be tracked in turn,
        by the state resource it references.

        State is kept as a single JSON document, patched in place on
        each state event, rather than as the event log that
        `nixops4-resources-local`'s `state_file` writes.
      '';
      requireState = true;
      inputs = {
        options = {
          name = mkOption {
            type = types.str;
            description = ''
              Path of the JSON document holding the stored state.
            '';
          };
        };
      };
      outputs = {
        options = { };
      };
    };
  };
}
