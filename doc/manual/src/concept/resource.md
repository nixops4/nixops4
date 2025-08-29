# Resource

A NixOps _resource_ is a unit of configuration that represents a certain real world entity, such as a virtual machine, a DNS record, or a NixOS installation.

Resources are the building blocks of a NixOps deployment.
They have _inputs_ and _outputs_ that can be connected to each other by passing Nix values around, so if resources are the bricks, Nix expressions are the mortar.
Both inputs and outputs are represented as Nix attributes. When you write a deployment expression, you create inputs by creating these attributes.
These attribute may in turn use other resources' outputs as their values.
An output is accessed by referring to the resource's attribute in the deployment expression.
An input may depend on zero or more outputs, but the references between resources must not form a cycle.

NixOps manages this data flow for you.

A [_resource provider_](../resource-provider/index.md) implements the operations that create, update, and delete the real world entity that the resource represents.

## State Management

Some resources need to maintain state between deployments. For example, a resource that generates a unique ID or stores configuration that should persist across updates. Resource types can declare whether they require state management through the `requireState` field.

When a resource type has `requireState = true`:
- The resource must specify a `state` handler (typically a reference to another resource that manages the state storage)
- NixOps will validate that the state handler is properly configured
- The resource provider can store and retrieve persistent state through this handler

When a resource type has `requireState = false`:
- The resource does not need a state handler
- The resource is stateless and can be idempotently recreated from its inputs

## Resource Dependencies

Resources can depend on each other in two different ways:

### Output → Input Dependencies

Most dependencies between resources involve one resource's output becomes another resource's input.
The resource graph topology is static:

```nix
{ resources, ... }:
{
  resources.database = {
    type = "postgresql_database";
    inputs.name = "myapp";
  };

  resources.user = {
    type = "postgresql_user";
    inputs.database = resources.database.name;
  };
}
```

### Structural Dependencies

The resource graph topology itself depends on resource outputs. Sub-deployment attributes are computed dynamically.

```nix
{ lib, resources, ... }:
{
  resources.settings = {
    imports = [ ./settings-resource.nix ];
    inputs.location = "https://panel.example.com/config.json";
  };

  deployments = {
    clients.deployments =
      # Structural dependency: the deployments structure under clients depends on the outputs of the settings resource
      lib.mapAttrs
        (clientName: clientConfig: {
          imports = [ ./client.nix ];
          clientConfig = clientConfig;
        })
        resources.settings.clients;

    shared = {
      resources = {
        # Resources here would not be structural dependencies
      }
      # This conditional resource is also a structural dependency on the settings resource
      // lib.optionalAttrs resources.settings.logging.enable {
        log_token = {
          # ...
        };
      };
    };
  };
}
```

Structural dependencies can only refer to `resources` attrsets that can be evaluated independently of the affected structure, so sub-`deployments` are required.
Specifically:
- A structural dependency in `resources` can not refer to any of the resources contained within that resource set.
- Similarly a structural dependency in `deployments` can not be self-referential either.
- It *is* possible to refer to the parent deployment or its other descendant deployments, assuming no mutual structural dependencies.
