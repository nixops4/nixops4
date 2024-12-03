{ self, ... }: {
  perSystem = { config, pkgs, inputs', ... }: {
    packages.manual = pkgs.callPackage ./package.nix {
      inherit (config.packages)
        nixops4
        nixops4-resource-runner
        manual-deployment-option-docs-md
        ;
      # https://github.com/NixOS/nixpkgs/issues/359286
      json-schema-for-humans = inputs'.nixpkgs-old.legacyPackages.json-schema-for-humans;
    };
    packages.manual-deployment-option-docs-md = pkgs.callPackage ./deployment-option-docs-md.nix { inherit self; };
    checks.manual-links = pkgs.callPackage ./test.nix { site = config.packages.manual; };
  };
}
