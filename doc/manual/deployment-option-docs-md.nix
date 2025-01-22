{ lib, self, nixosOptionsDoc, ... }:
let
  configuration =
    self.lib.evalDeployment
      {
        modules = [
          ../../nix/deployment/base-modules.nix
          hideModuleArgs
        ];
        specialArgs = { };
      }
      {
        resources = { };
        resourceProviderSystem = "<resourceProviderSystem>";
      };
  hideModuleArgs = { lib, ... }: {
    options = {
      _module.args = lib.mkOption {
        visible = false;
      };
    };
  };
  docs = nixosOptionsDoc {
    inherit (configuration) options;
    transformOptions = transformOption;
  };
  sourcePathStr = "${self.outPath}";
  baseUrl = "https://github.com/nixops4/nixops4/tree/main";
  sourceName = "nixops4";
  transformOption = opt: opt // {
    declarations = lib.concatMap
      (decl:
        if lib.hasPrefix sourcePathStr (toString decl)
        then
          let subpath = lib.removePrefix sourcePathStr (toString decl);
          in [{ url = baseUrl + subpath; name = sourceName + subpath; }]
        else [ ]
      )
      opt.declarations;
  };
in
docs.optionsCommonMark.overrideAttrs {
  extraArgs = [
    # Align with NixOS HTML manual and flake-parts
    "--anchor-prefix"
    "opt-"
    "--anchor-style"
    "legacy"
  ];
}
