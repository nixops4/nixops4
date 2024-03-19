# Component architecture

## Context

As previously discussed, NixOps 4's architecture has great similarity with Nix's architecture.

The goal of this document is to outline the components and layers that make up NixOps 4.

## Decision

At a high level, `nixops4` is an executable that realises its extensions through Nix, starts them, and interacts with them.

`nixops4` individually has a layered application architecture, with the following layers:

- The NixOps CLI, analogous to Nix's `src/nix`
  - Implements the command line interface
- The NixOps evaluator process
  - Implements an interface provided by the NixOps core library
  - Exists as a separate process, to improve the robustness of the system
    - Data loss following resource creation can have a real world cost, especially if not reported to the user.
- The NixOps core library, analogous to Nix's `libstore`, and using it, but not `libexpr`.
  - Implements the interaction with resource implementations
  - Coordinates the interactions between resources and the evaluator

During prototyping, the NixOps evaluator process might be incorporated into the CLI, to simplify the development process.

Besides the `nixops4` program, the following artifacts will be provided by the NixOps project:
- Schemas for the various interprocess communications, in an IDL such as JSON Schema.
- Optionally, "SDK" libraries that extend these schemas with generic functionality, as applicable.

Users of `nixops4` - the NixOps 4 ecosystem, supported by the NixOps project - will provide the following components:
- Resource providers for
  - Cloud providers
  - Wrappers for other systems, such as Terraform/OpenTofu
  - Interactions with container runtimes
  - Local command invocation
  - Cryptographic key generation
  - Secrets retrieval
- Expressions that define
  - Adapters towards resources
  - Compositions of resources ("modules" in Terraform parlance)
  - Off the shelf deployments

Generally these will be developed and distributed together, as the expression couple tightly with the resource providers.
Decoupling / abstraction is possible by informally defining interfaces.
For instance, we should work towards a standard _expression_ interface for declaring a cloud instance that runs a NixOS configuration, so that larger expressions can deploy cloud agnostic applications to any cloud.
Similar interfaces should emerge for other resource types, such as object storage, databases, and container runtimes.

