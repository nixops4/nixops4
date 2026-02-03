# Unified Component Model

## Context

Currently, nixops4 distinguishes between:
- **Resources**: Units managed by providers (e.g., a DNS record, an EC2 instance)
- **Nested deployments**: Units containing other resources and deployments

This distinction creates friction:
- Different syntax for addressing resources vs deployments
- Refactoring a resource into a sub-deployment (or vice versa) breaks all references
- Users must understand the implementation structure, not just the logical structure

## Decision

Unify resources and nested deployments into a single abstraction: **components**.

A component is either:
- **Resource component**: Defines `resource` - wraps a provider-managed resource
- **Composite component**: Defines `members` - contains other components

The path `foo.bar.baz` uniformly means "the member `baz` inside member `bar` inside member `foo`". The final component may be either a resource or composite; the preceding components must be composites.

Key terminology:
- **Component**: The unified type
- **Member**: The relationship (a component is a member of its parent)
- **Root**: The top-level component, created by `mkRoot`. Not a member, having no parent component.

## Consequences

- Uniform path-based addressing for all components
- CLI accepts member paths directly: `nixops4 apply foo bar.baz`
- Refactoring between resource and composite no longer breaks references
- `members` module argument provides sibling access at each level
- Providers can be defined at any component level, not just root
