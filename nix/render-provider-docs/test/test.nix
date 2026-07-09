# Run with: nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).render-provider-docs
{
  renderProviderDocs,
  lib,
  runCommand,
}:
let
  # Test provider module with various resource types
  testProviderModule = {
    name = "Test Provider";
    description = "A test provider for documentation generation testing";
    sourceBaseUrl = "https://github.com/nixops4/nixops4/tree/main";
    sourceName = "nixops4";
    resourceTypes = {
      # Simple resource with no inputs/outputs
      simple = {
        description = "A simple test resource with no configuration";
        requireState = false;
        inputs = { };
        outputs = { };
      };

      # Resource with inputs and outputs
      configured = {
        description = ''
          A test resource with configuration options.

          This resource demonstrates input and output documentation.
        '';
        requireState = false;
        inputs = {
          options = {
            name = lib.mkOption {
              type = lib.types.str;
              description = "The name of the resource";
            };
            count = lib.mkOption {
              type = lib.types.int;
              default = 1;
              description = "Number of instances";
            };
          };
        };
        outputs = {
          options = {
            id = lib.mkOption {
              type = lib.types.str;
              description = "Identifier provided by remote API";
            };
          };
        };
      };

      # Stateful resource
      stateful = {
        description = "A test resource that requires state persistence";
        requireState = true;
        inputs = {
          options = {
            value = lib.mkOption {
              type = lib.types.str;
              description = "The value to store";
            };
          };
        };
        outputs = {
          options = {
            stored_value = lib.mkOption {
              type = lib.types.str;
              description = "The stored value";
            };
          };
        };
      };
    };
  };

  # Render docs for the test provider
  renderedDocs = renderProviderDocs {
    module = testProviderModule;
  };

  # Expected directory containing golden files
  expectedDir = ./expected;

in
runCommand "render-provider-docs-golden"
  {
    expected = expectedDir;
    actual = renderedDocs;
  }
  ''
    if ! diff -U3 -r -- "$actual" "$expected"; then
      echo
      echo 'Rendered provider docs do not match the expected golden output.'
      echo "+: expected,   at $expected"
      echo "-: unexpected, at $actual"
      echo
      echo 'If these changes look acceptable, run'
      echo '  nix/render-provider-docs/test/adopt-all-changes.sh'
      exit 1
    fi
    touch "$out"
  ''
