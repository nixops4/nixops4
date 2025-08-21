{
  inputs = {
    nixpkgs.url = "@nixpkgs@";
    nixops4.url = "@nixops4@";
    flake-parts.url = "@flake-parts@";
    null.url = "@null@";
    binaries.url = "@binaries@";
    binaries.flake = false;
    prebuilt-nix-cargo-integration.url = "@prebuilt-nix-cargo-integration@";
    prebuilt-nix-cargo-integration.inputs.binaries.follows = "binaries";
    nixops4.inputs.flake-parts.follows = "flake-parts";
    nixops4.inputs.nix-cargo-integration.follows = "prebuilt-nix-cargo-integration";
    nixops4.inputs.nix.follows = "null";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";
  };

  outputs = inputs: inputs.nixops4;
}
