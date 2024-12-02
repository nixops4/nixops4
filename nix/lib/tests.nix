/**
  nix-unit tests for `nixops4.lib`
  nix-unit --flake .#tests
 */
{ lib, self, system }:
{
  "test basically nothing" = {
    expr = true;
    expected = true;
  };
}
