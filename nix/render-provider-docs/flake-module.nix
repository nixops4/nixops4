{ self, ... }:
{
  perSystem =
    {
      config,
      pkgs,
      ...
    }:
    {
      # Documentation: ../lib/lib.nix
      builders.renderProviderDocs =
        {
          module,
        }:
        pkgs.callPackage ./package.nix {
          inherit self;
          providerModule = module;
        };
      checks.render-provider-docs = pkgs.callPackage ./test/test.nix {
        inherit (config.builders) renderProviderDocs;
      };
    };
}
