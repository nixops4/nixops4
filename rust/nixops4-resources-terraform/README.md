# NixOps4 Terraform Provider Integration

This package provides a generic adapter that allows NixOps4 to use existing Terraform providers as resource backends. It automatically translates Terraform provider schemas into NixOps4 resource types.

## Quick Start

```nix
# In your deployment configuration
{
  providers.postgresql = self.lib.tfProviderToModule {
    tfProvider = pkgs.terraform-providers.postgresql;
  };
  
  resources.myDatabase = {
    type = providers.postgresql.postgresql_role;
    inputs = {
      # Resource-specific attributes
      name = "app-user";
      login = true;
      create_database = true;
      
      # Provider connection settings (note the tf-provider- prefix)
      tf-provider-host = "localhost";
      tf-provider-username = "admin";
      tf-provider-password = config.secrets.postgresPassword;
    };
  };
}
```

## Schema Translation Overview

The adapter automatically converts Terraform provider schemas into NixOps4 resource definitions:

### Resource Types
- **Terraform resources** → NixOps4 resource types (e.g., `postgresql_role`)
- **Terraform data sources** → NixOps4 resource types with `data-source-` prefix (e.g., `data-source-postgresql_schemas`)

### Attribute Translation

Terraform provider schemas define attributes with three key properties:
- `required`: Must be provided by user
- `optional`: Can be provided by user
- `computed`: Set by the provider (API responses, generated values)

These map to NixOps4 as follows:

| Terraform Schema | NixOps4 Input | NixOps4 Output | Description |
|------------------|---------------|----------------|-------------|
| `required: true` | ✅ Required | ❌ | User must provide |
| `optional: true, computed: false` | ✅ Optional | ❌ | User can provide |
| `computed: true, optional: false` | ❌ | ✅ Read-only | Provider sets |
| `optional: true, computed: true` | ✅ Optional | ✅ | User can provide OR provider computes |

## Provider Configuration

### The Prefixing System

To avoid naming collisions between resource attributes and provider configuration, all provider-level settings are prefixed with `tf-provider-`:

```nix
{
  # ❌ This would conflict if both resource and provider had a "host" attribute
  host = "localhost";
  
  # ✅ Provider configuration is clearly distinguished
  tf-provider-host = "localhost";
  tf-provider-username = "admin";
  tf-provider-database = "myapp";
}
```



## Resource Examples

### PostgreSQL Role

```nix
resources.appUser = {
  type = providers.postgresql.postgresql_role;
  inputs = {
    # Resource configuration
    name = "app-user";

    # PostgreSQL provider
    tf-provider-host = "localhost";
    tf-provider-port = 5432;
    tf-provider-username = "admin";
    tf-provider-password = config.secrets.pgPassword;
  };
}
```

## Data Sources

Data sources become resource types with a `data-source-` prefix:

```nix
resources.schemas = {
  type = providers.postgresql."data-source-postgresql_schemas";
  inputs = {
    tf-provider-host = "localhost";
    tf-provider-username = "readonly";
  };
};

# Access the data
outputs = {
  allSchemas = resources.schemas.outputs.schemas;
};
```

## Type Mapping

Terraform types map to NixOS module system types:

| Terraform Type | NixOS Type | Example |
|---------------|------------|---------|
| `"string"` | `types.str` | `"hello"` |
| `"bool"` | `types.bool` | `true` |
| `"number"` | `types.int` | `42` |
| Complex types | `types.raw` | `["list", "of", "items"]` |

Complex types are planned to be modeled properly.

## Implementation Details

### Schema Extraction

The adapter uses the Terraform provider's schema endpoint to automatically discover:
- Available resource types and their attributes
- Provider configuration options
- Attribute types, descriptions, and requirements
- Computed vs. input attributes

#### Import From Derivation

This currently happens through import from derivation (IFD).

NixOps4's purpose is to interleave evaluation and other tasks, and this is ok:
- NixOps4 enables concurrent evaluation to continue when blocked on a yet unavailable value
- Resources aren't triggered by accident, whereas IFD is sometimes used unnecessarily, where a derivation-derivation dependency could have done the job.

The use of IFD can be replaced by changing NixOps4 to support the required computation through a different path, and doing so improves on both points.
- By blocking NixOps4 instead of the evaluator during the schema-to-modules build, the evaluator can continue. NixOps4 is highly concurrent, so only that specific task and its dependents are blocked; the rest continues
- You'll be able to re-disable IFD, so that you can spot accidental IFDs easily; not just in Nix but also in NixOps, increasing your "linting" coverage.

### Provider Execution

Resources are executed using the `nixops4-resources-terraform` binary which:
1. Starts the Terraform provider as a gRPC service
2. Translates NixOps4 resource operations to Terraform provider calls  
3. Handles the provider handshake and protocol negotiation
4. Manages provider configuration and resource lifecycle

### State Management

- Uses NixOps4's stateful resource system (`requireState = true`)
- State is managed independently per resource
- Supports incremental updates and proper resource cleanup

## Limitations

- Complex nested types currently use `types.raw` (may need manual type definitions for better UX)
- Provider-specific defaults are not automatically applied
- Error messages reference Terraform concepts that may be unfamiliar to Nix users

## Troubleshooting

### Common Issues

1. **Missing tf-provider- prefix**:
   ```
   Error: The option 'resources.myResource.inputs.host' does not exist
   ```
   Solution: Use `tf-provider-host` instead of `host` for provider configuration.

2. **Required attribute missing**:
   ```
   Error: The option 'resources.myResource.inputs.name' was accessed but has no value defined
   ```
   Solution: Provide all required attributes according to the Terraform provider schema.

3. **Computed attribute in inputs**:
   ```
   Error: You cannot set 'id' because it is read-only
   ```
   Solution: Remove computed-only attributes from inputs; they will appear in outputs.

### Debugging

To inspect the translated schema:
```nix
# Check available resource types
nix eval .#deployments.myDeployment.getProviders.x86_64-linux.passthru.config.providers.myProvider.resourceTypes --apply builtins.attrNames

# Inspect a specific resource type
nix eval .#deployments.myDeployment.getProviders.x86_64-linux.passthru.config.providers.myProvider.resourceTypes.my_resource
```

## Contributing

This is an MVP implementation. Future improvements could include:

- Better type mapping for complex Terraform types
- Support for provider-specific defaults and validation
- Enhanced error messages with Nix-friendly terminology
- Helper functions to reduce provider configuration repetition across resources:
  This is actually more of a nixops4 nix expressions change
  ```nix
  # Proposed helper to apply common provider config
  { providers, ... }: {
  {
    providers.my-pg = {
      imports = [ (self.lib.tfProviderToModule {
        tfProvider = pkgs.terraform-providers.postgresql;
      }) ];
      common = { ... }: {
        # _class = "nixops4Resource";
        inputs = {
          tf-provider-host = "db.example.com";
          tf-provider-username = "admin";
        };
      };
    };
    resources = {
      appUser.type = providers.my-pg.postgresql_role;
      appUser.inputs.name = "app-user";
      
      readOnlyUser.type = providers.my-pg.postgresql_role; 
      readOnlyUser.inputs.name = "readonly";
    };
  };
  ```
