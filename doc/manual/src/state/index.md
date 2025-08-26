# State

NixOps4 supports [resources](../concept/resource.md) that are stateful. That is to say, they have outputs that need to be remembered.
Furthermore, state helps with the removal of resources whose declaration was deleted, even if they were stateless in principle.

The storage of state information is handled by specialized resources that implement some extra operations.
We refer to these as state provider resources.
State provider resources implement additional operations ([`state_read` and `state_event`](../resource-provider/interface.md#state-operations)) beyond the standard create/update operations.

A state provider resource, like any other resource, may be stateful or stateless in terms of their own lifecycle.

For instance, the "local" provider has a `state_file` resource type, which is stateless, because all identifying information is provided as inputs, and its creation does not produce any identifiers or other significant information that is suitable for use in a declarative deployment expression.
This makes it suitable for bootstrapping your deployment state.
You can use it to store connection details for a more sophisticated state provider resource (like a cloud service) that your team can access. <!-- to be proven out -->

For the rest of this page, we'll focus on `state_file`. Other state providers provide compatible behavior, but have unique approaches to storage.

## State file structure

The state file contains a sequence of JSON objects, each representing a state event:

<!-- Tested in ../../../../test/json-schema.nix -->
<!-- Tested in ../../../../test/nixops4-resources-local.nix -->
```json
{{#include snippets/state-file.json}}
```

Each event includes:
- `index`: Sequential event number starting from 0
- `meta`: Event metadata including timestamp and operation details
- `patch`: JSON Patch operations to apply to reconstruct the state

State events use [JSON Patch (RFC 6902)](https://tools.ietf.org/html/rfc6902) operations to record incremental changes to the state. This enables efficient state updates and provides an audit trail of all state modifications.

The first event, index 0, always initializes the state structure, as a matter of separation of concerns, and perhaps as a recognizable "file type marker", although we don't make exact promises about the precise contents of this first object.

After adding resources to the state, reading and resolving the state would return something like:

<!-- Tested in ../../../../test/json-schema.nix -->
<!-- Tested in ../../../../test/nixops4-resources-local.nix -->
```json
{{#include snippets/resolved-state.json}}
```

The state structure follows the [NixOps4 state schema](../schema/state-v0.md).

## Using state providers

State providers are configured in your deployment using the `state` attribute on stateful resources:

<!-- TODO: put a piece of real world deployment here -->
```nix
{{#include snippets/deployment-config.nix}}
```

For testing state provider resources, see [Testing State Resources](../resource-provider/testing.md#example-testing-state-resources).
For implementation details, see the [Resource Provider Interface](../resource-provider/interface.md#state-operations).
