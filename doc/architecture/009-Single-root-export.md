# Single Root Export

## Context

Currently, nixops4 expects flakes to export multiple named deployments:

```nix
{
  outputs.nixops4Deployments.production = mkDeployment { ... };
  outputs.nixops4Deployments.staging = mkDeployment { ... };
}
```

The CLI then selects a deployment by name: `nixops4 apply production`.

With the unified component model, components can be nested arbitrarily deep. This makes named deployments at the flake level redundant - the same organization can be achieved within a single component tree using member paths.

## Decision

Replace `outputs.nixops4Deployments.<name>` with a single `outputs.nixops4` root component.

```nix
{
  outputs.nixops4 = mkRoot {
    modules = [ ./infrastructure.nix ];
  };
}
```

Selection happens via member paths rather than deployment names:

```
nixops4 apply                    # apply entire root
nixops4 apply production         # apply the production subtree
nixops4 apply staging.database   # apply a specific member
```

## Consequences

- Simpler flake interface: one export instead of an attrset
- Path-based selection subsumes named deployment selection
- `mkDeployment` becomes `mkRoot` (only the root needs a wrapper function)
- Consistent mental model: everything is a component, addressed by path
- Enables references between what would otherwise be isolated top-level deployments
- Separation can still be achieved with an imports-into-submodule pattern if users want to enforce it
- A boolean option can disallow deploying a component as a whole, useful at root level to prevent accidental full deployments
