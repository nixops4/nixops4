{ self, ... }:
{
  imports = [
    ../../nix/flake-parts/builders.nix
    ../../nix/render-provider-docs/flake-module.nix
  ];
  perSystem =
    { config
    , pkgs
    , inputs'
    , ...
    }:
    {
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
      apps.open-manual.meta.description = "Open the NixOps4 manual in your browser";
      packages.manual = pkgs.callPackage ./package.nix {
        inherit (config.packages)
          nixops4
          nixops4-resource-runner
          manual-deployment-option-docs-md
          manual-provider-option-docs-md-local
          ;
      };
      packages.manual-deployment-option-docs-md = pkgs.callPackage ./deployment-option-docs-md.nix {
        inherit self;
      };
      packages.manual-provider-option-docs-md-local = config.builders.renderProviderDocs {
        module = self.modules.nixops4Provider.local;
      };
      checks.manual-links = pkgs.callPackage ./test.nix { site = config.packages.manual; };
    };
}
