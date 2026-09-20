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

### Changing a Declaration

What NixOps can do depends on whether the resource state is tracked or not, i.e. whether `state` is set.

When a resource is tracked, NixOps knows the state as it was last applied.
This means it can hand the provider both this previous state and the declared intended state,
so that the provider can carry out the changes with full information.
Destroying follows the same idea: it removes what was created rather than whatever the declaration names now.

A resource with no state has only the declaration, because the state of it was not recorded.
This means that the provider can only perform an "upsert"-style operation: create if it doesn't exist yet, and update if it does.
Often this is good enough, but it breaks down with mutable identities.
For instance, the file `name` property may have changed, but without tracking the previous state, the provider can not know to rename it.
Instead, you will have a new resource under the new name, and the old resource sticks around, without a way for NixOps to remove it or tell you that it's still there.
This can be a costly affair, so virtually all cloud provisioning resources require state and can not be used without it (even if in some cases they could work statelessly).
Since NixOps4 can not automatically know when to delete an untracked resource, such deletions must be performed with `nixops4 destroy` before removing their declarations.

So if you use stateless deployments, make sure the names you use are unchanging, or if they do change, you manage the change yourself.

### Skipping Unchanged Resources

When NixOps applies a stateful resource whose inputs haven't changed since the last apply, it considers the resource up to date and does not perform the update operation.
This keeps deployments fast by only invoking providers when inputs actually change.

This relies on two assumptions:

1. **No out-of-band changes**: the resource has not been modified outside of NixOps.
   Drift detection and reconciliation [will be](https://github.com/nixops4/nixops4/issues/161) NixOps responsibilities, but they are not part of the normal `apply` flow, because that would unnecessarily slow down deployments.
   If your deployment needs drift detection, it should run continuously rather than on update, so that problems are detected promptly.
2. **No significant side effects**: the update operation does not perform essential work beyond bringing the resource to its desired state.
   Concerns such as lease renewal are outside the scope of resource providers and should be handled by services or scheduled tasks instead.

## Resource Dependencies

Resources can depend on each other in two different ways:

### Output → Input Dependencies

Most dependencies between resources involve one resource's output becomes another resource's input.
The resource graph topology is static:

```nix
{ members, ... }:
{
  members.database = {
    type = "postgresql_database";
    inputs.name = "myapp";
  };

  members.user = {
    type = "postgresql_user";
    inputs.database = members.database.outputs.name;
  };
}
```

### Structural Dependencies

The component graph topology itself depends on resource outputs. Sub-component attributes are computed dynamically.

```nix
{ lib, members, ... }:
{
  members.settings = {
    imports = [ ./settings-resource.nix ];
    inputs.location = "https://panel.example.com/config.json";
  };

  members.clients.members =
    # Structural dependency: the members structure under clients depends on the outputs of the settings resource
    lib.mapAttrs
      (clientName: clientConfig: {
        imports = [ ./client.nix ];
        clientConfig = clientConfig;
      })
      members.settings.outputs.clients;

  members.shared.members = {
    # Static members here would not be structural dependencies
  }
  # This conditional member is also a structural dependency on the settings resource
  // lib.optionalAttrs members.settings.outputs.logging.enable {
    log_token = {
      # ...
    };
  };
}
```

Structural dependencies can only refer to `members` attrsets that can be evaluated independently of the affected structure.
Specifically:
- A structural dependency in `members` cannot refer to any of the members contained within that member set.
- It *is* possible to refer to sibling or parent members, assuming no mutual structural dependencies.
