{ renderProviderDocs
, lib
, testers
,
}:
let
  # Test provider module with various resource types
  testProviderModule = {
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
# TODO after https://github.com/NixOS/nixpkgs/pull/436528 add message to explain and carefully hint at ./adopt-all-changes.sh
  #      - require thorough review of the diff
testers.testEqualContents {
  assertion = "Rendered provider docs match expected golden output";
  expected = expectedDir;
  actual = renderedDocs;
}
