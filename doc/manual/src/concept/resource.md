# Resource

A NixOps _resource_ is a unit of configuration that represents a certain real world entity, such as a virtual machine, a DNS records, or a NixOS installation.

Resources are the building blocks of a NixOps deployment, with Nix expressions as the mortar that holds them together.
They have _inputs_ and _outputs_ that can be connected to each other by passing Nix values around.
Both inputs and outputs are represented as Nix attributes. When you write a deployment expression, you create inputs by creating these attributes.
These attribute may in turn use other resources' outputs as their values.
An output is accessed by referring to the resource's attribute in the deployment expression.
An input may depend on zero or more outputs, but the dependencies between resources must not be cyclic.

NixOps manages this data flow for you.

A [_resource provider_](../resource-provider/index.md) implements the operations that create, update, and delete the real world entity that the resource represents.
