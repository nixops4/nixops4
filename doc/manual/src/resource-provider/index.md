# Resource Provider

A _resource provider_ is the component that is responsible for carrying out the operations that create, update, and delete the real world entity that a resource represents.

This section of the manual focuses on the implementation of resource providers.
It is intended for developers who need to write custom resource providers for NixOps4.
This is not always necessary, as a suitable resource provider may already exist, or in other cases it is possible to build a module that achieves the desired effect using existing resource providers.

## Resource Type Declaration

When implementing a resource provider, you define resource types that specify:
- **inputs**: The configuration options that users provide
- **outputs**: The values that the resource exposes to other resources
- **requireState**: Whether the resource needs persistent state management

### State Requirements

The `requireState` field in a resource type declaration indicates whether resources of this type need to maintain state between deployments:

Example stateless resource:
```nix
{
  resourceTypes.file = {
    description = "File on the local file system";
    requireState = false;
    inputs = { /* ... (options) */ };
    outputs = { /* ... (options) */ };
  };
}
```

Example stateful resource:
```nix
{
  resourceTypes.reverse_proxy = {
    description = "Instance of a CloudTM managed reverse proxy";
    requireState = true;
    inputs = { /* ... (options) */ };
    outputs = { /* ... (options) */ };
  };
}
```

When `requireState = true`, users must configure a `state` handler for the resource, referencing a state storage resource. NixOps will validate this requirement at evaluation time.
