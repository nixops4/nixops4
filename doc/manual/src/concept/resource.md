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
