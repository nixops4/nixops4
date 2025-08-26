# Tests in: ../render-provider-docs/test/test.nix
{ lib, ... }:
let
  inherit (lib) mkOption types;
in
{
  options = {
    name = mkOption {
      type = types.str;
      description = ''
        The display name of the resource provider.
      '';
    };

    description = mkOption {
      type = types.str;
      description = ''
        A description of what the resource provider does.
        This will be displayed on the provider's documentation index page.
      '';
    };

    sourceBaseUrl = mkOption {
      type = types.str;
      description = ''
        Base URL for linking to the provider's source code.
        Used in generated documentation to create links to option declarations.
      '';
    };

    sourceName = mkOption {
      type = types.str;
      description = ''
        Name of the source repository or project.
        Used in generated documentation link display text.
      '';
    };
  };
}
