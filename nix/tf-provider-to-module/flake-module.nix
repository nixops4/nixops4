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
      builders.generateCommonTfSchema =
        {
          tfProvider,
        }:
        pkgs.runCommand "terraform-provider-schema"
          {
            nativeBuildInputs = [
              config.packages.nixops4-resources-terraform-release
            ];
          }
          ''
            # Create output directory
            mkdir -p $out

            # Extract schema from terraform provider
            nixops4-resources-terraform schema --provider-path "${self.lib.providerPath tfProvider}" > $out/schema.json
          '';
    };
}
