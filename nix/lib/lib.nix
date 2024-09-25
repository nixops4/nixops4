{
  # Nixpkgs lib
  lib
, # This nixops4 flake
  self
, # withSystem of the nixops4 flake
  # https://flake.parts/module-arguments#withsystem
  selfWithSystem
,
}:

# Flake `lib` output attribute: user facing functions for declaring deployments, etc.
{
  deployment = import ./deployment.nix { inherit lib self selfWithSystem; };
}
