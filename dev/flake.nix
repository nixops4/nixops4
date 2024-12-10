{
  description = "dependencies only";
  inputs = {
    pre-commit-hooks-nix.url = "github:cachix/pre-commit-hooks.nix";
    pre-commit-hooks-nix.inputs.nixpkgs.follows = "";
    hercules-ci-effects.url = "github:hercules-ci/hercules-ci-effects";
    nix-unit.url = "github:nix-community/nix-unit";
  };
  outputs = { ... }: { };
}
