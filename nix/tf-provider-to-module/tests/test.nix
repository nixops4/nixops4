# Run with:
#   nix-unit --flake .#tests.systems.<system>.tf-provider-to-module
# or, slower:
#   nix build .#checks.<system>.nix-unit
{
  lib,
  self,
  selfWithSystem,
  system,
}:

let
  # Load test data
  postgresqlSchema = builtins.fromJSON (
    builtins.readFile ./terraform-provider-postgresql-schema.json
  );

  # Mock terraform provider package for testing
  mockTfProvider = {
    pname = "terraform-provider-postgresql";
    stdenv.hostPlatform.system = system;
    provider-source-address = "cyrilgdn/postgresql";
    version = "1.25.0";
    GOOS = "linux";
    GOARCH = "amd64";
    outPath = "/nix/store/mock-terraform-provider-postgresql";
    meta.mainProgram = "terraform-provider-postgresql";
  };

  # Helper function to evaluate a provider module (similar to render-provider-docs)
  evalProviderModule =
    providerModule:
    lib.evalModules {
      modules = [
        ../../provider/provider.nix
        { options._module.args = lib.mkOption { internal = true; }; }
        providerModule
      ];
      specialArgs = {
        resourceProviderSystem = throw "Test should not depend on resourceProviderSystem";
      };
    };

  # Test the translation function
  translatedModuleFunction = self.lib.tfCommonSchemaToModule mockTfProvider postgresqlSchema;

  # Evaluate the translated module
  providerEval = evalProviderModule translatedModuleFunction;

  # Create a test deployment with actual resources using the translated provider
  testDeployment = self.lib.mkRoot {
    modules = [
      (
        {
          config,
          providers,
          members,
          ...
        }:
        {
          providers.testProvider = translatedModuleFunction;
          members.testRole = {
            type = providers.testProvider.postgresql_role;
            inputs = {
              # Required attribute
              name = "test-role-name";
              # Optional bool attributes
              login = true;
              create_database = false;
              # Optional number attribute
              connection_limit = 10;
              # Optional list attribute
              search_path = [
                "public"
                "app_schema"
              ];
              # Provider attribute
              tf-provider-host = "localhost";
            };
          };
        }
      )
    ];
  };

  # Evaluate the test deployment (as NixOps4 would)
  # Mock resource outputs as would be discovered by the provider
  testDeploymentEval = testDeployment.rootFunction {
    resourceProviderSystem = system;
    outputValues = {
      testRole = {
        # Only computed attribute for postgresql_role based on schema
        id = "test-role";
      };
    };
  };

in

{
  # Test that function exists and returns expected structure
  testFunctionExists = {
    expr = builtins.isFunction self.lib.tfCommonSchemaToModule;
    expected = true;
  };

  # Test provider name is set correctly
  testProviderName = {
    expr = providerEval.config.name;
    expected = "terraform-provider-postgresql";
  };

  # Test provider description value
  testProviderDescription = {
    expr = providerEval.config.description;
    expected = "terraform-provider-postgresql";
  };

  # Test exact resource type count from schema
  testResourceTypeCount = {
    expr = builtins.length (builtins.attrNames providerEval.config.resourceTypes);
    expected =
      builtins.length (builtins.attrNames postgresqlSchema.resource_schemas)
      + builtins.length (builtins.attrNames postgresqlSchema.data_source_schemas);
  };

  # Test resource provider type is correct
  testResourceProviderType = {
    expr = providerEval.config.resourceTypes.postgresql_role.provider.type;
    expected = "postgresql_role";
  };

  # Test data source provider type has correct prefix
  testDataSourceProviderType = {
    expr = providerEval.config.resourceTypes."data-source-postgresql_schemas".provider.type;
    expected = "get-postgresql_schemas";
  };

  # Test resource state requirement
  testResourceRequiresState = {
    expr = providerEval.config.resourceTypes.postgresql_role.requireState;
    expected = true;
  };

  # Test resource description value
  testResourceDescription = {
    expr = providerEval.config.resourceTypes.postgresql_role.description;
    expected = "Terraform resource postgresql_role";
  };

  # Test provider executable exact value
  testProviderExecutable = {
    expr = providerEval.config.resourceTypes.postgresql_role.provider.executable;
    expected = lib.getExe (
      selfWithSystem system ({ config, ... }: config.packages.nixops4-resources-terraform-release)
    );
  };

  # Test provider args exact values
  testProviderArgs = {
    expr = providerEval.config.resourceTypes.postgresql_role.provider.args;
    expected = [
      "run"
      "--provider-exe"
      (self.lib.providerPath mockTfProvider)
    ];
  };

  # Test explicitly set resource inputs
  testResourceInputs =
    let
      inputs = testDeploymentEval.members.testRole.resource.inputs;
    in
    {
      expr = {
        inherit (inputs)
          name
          login
          create_database
          connection_limit
          search_path
          tf-provider-host
          ;
      };
      expected = {
        name = "test-role-name";
        login = true;
        create_database = false;
        connection_limit = 10;
        search_path = [
          "public"
          "app_schema"
        ];
        tf-provider-host = "localhost";
      };
    };

  # Test that unset optional inputs are passed as null
  testOptionalInputsAreNull =
    let
      inputs = testDeploymentEval.members.testRole.resource.inputs;
    in
    {
      expr = {
        inherit (inputs) assume_role tf-provider-aws_rds_iam_auth;
      };
      expected = {
        assume_role = null;
        tf-provider-aws_rds_iam_auth = null;
      };
    };

  # Test that computed attributes appear in the output skeleton
  testResourceOutputsSkeleton = {
    expr = testDeploymentEval.members.testRole.resource.outputsSkeleton;
    expected = {
      id = { };
    };
  };
}
