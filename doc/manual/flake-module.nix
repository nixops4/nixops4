{ self, ... }: {
  perSystem = { config, pkgs, inputs', ... }: {
    apps.open-manual.program = pkgs.writeScriptBin "open-nixops4-manual" ''
      #!${pkgs.runtimeShell}
      manual='${config.packages.manual.index}'
      echo >&2 "Built manual: $manual"
      echo >&2 "Opening the built manual in a browser..."
      echo >&2 ""
      echo >&2 " 💡 For quick iteration, use"
      echo >&2 ""
      echo >&2 "  cd doc/manual; make open"
      echo >&2 ""

      if type xdg-open &>/dev/null; then
        xdg-open "$manual"
      else
        open "$manual"
      fi
    '';
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
