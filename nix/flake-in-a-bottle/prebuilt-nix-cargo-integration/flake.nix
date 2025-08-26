{
  # inputs.binaries.url = "file:///dev/null";
  inputs.binaries.flake = false;
  outputs =
    inputs@{ ... }:
    {
      flakeModule =
        { lib, ... }:
        let
          inherit (lib) mkOption types;
        in
        {
          perSystem =
            { lib, ... }:
            {
              options = {
                nci = mkOption {
                  type = types.anything;
                };
              };
              config.packages =
                let
                  binaries = builtins.fromJSON (builtins.readFile inputs.binaries);
                  toDerivation = path: {
                    type = "derivation";
                    outPath = path;
                  };
                in
                lib.mapAttrs (_: toDerivation) binaries;
              config.nci = lib.mkForce (
                throw "perSystem.nci is disabled, because we're supposed to use prebuilt packages in this context. If you see this error message, that means that some code is reading from the nci options, and either that code shouldn't do that, or that code should itself not have been invoked, etc."
              );
            };
        };
    };
}
