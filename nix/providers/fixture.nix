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
        by the state resource it references: it writes its own
        `privateState`, which NixOps4 must persist there and hand
        back on every later operation, including the `state_read`
        and `state_event` calls.

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
          new_private_state = mkOption {
            type = types.nullOr types.anything;
            default = null;
            description = ''
              Value the resource writes into its own private state on
              create and update.
            '';
          };
          expected_prior = mkOption {
            type = types.nullOr types.anything;
            default = null;
            description = ''
              When set, the resource asserts on update that the prior
              private state NixOps hands back equals this value.
            '';
          };
          expect_own_private_state = mkOption {
            type = types.nullOr types.anything;
            default = null;
            description = ''
              When set, the resource asserts that `state_read` and
              `state_event` receive this value as its own private
              state. Catches NixOps opening a stateful state resource as if
              it had never written private state.
            '';
          };
        };
      };
      outputs = {
        options = { };
      };
    };
    private_state_echo = {
      description = ''
        Round-trips the resource protocol's `privateState` slot for
        integration tests. On create, writes `new_private_state` as
        the resource's private state. On update, asserts the prior
        private state handed back by NixOps4 equals `expected_prior`
        (when set), then rotates to `new_private_state`.
      '';
      requireState = true;
      inputs = {
        options = {
          new_private_state = mkOption {
            type = types.nullOr types.anything;
            default = null;
            description = ''
              Value the provider writes into the resource's private
              state on this operation. When absent, the provider
              clears the resource's private state.
            '';
          };
          expected_prior = mkOption {
            type = types.nullOr types.anything;
            default = null;
            description = ''
              When set, the provider asserts the prior private state
              NixOps4 hands back equals this value. Catches NixOps4
              losing or corrupting the private-state persistence.
            '';
          };
        };
      };
      outputs = {
        options = { };
      };
    };
    undestroyable = {
      description = ''
        Creates and updates without doing anything; always fails to
        destroy. Lets a test watch a destroy fail and then see the
        resource still in state.
      '';
      requireState = true;
      inputs = {
        options = {
          reason = mkOption {
            type = types.str;
            default = "undestroyable by construction";
            description = ''
              Text the provider puts in the error it fails with.
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
