{
  description = "dependencies only";
  inputs = {
    pre-commit-hooks-nix.url = "github:cachix/pre-commit-hooks.nix";
    pre-commit-hooks-nix.inputs.nixpkgs.follows = "";
  };
  outputs = { ... }: { };
}
