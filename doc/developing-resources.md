
# Developing Resources

All NixOps4 really does is manage the information flows between resources, and between resources and the `nixops4` command.
The foundation of all useful functionality is provided by resources, which can be developed by anyone.

## Responsibilities

Just as when you're writing your own derivations with Nix, you have to abide by some rules.
Whereas Nix helps you a lot, by providing a build sandbox, and a functional language, NixOps4 solves a different problem.
While we get to keep the benefits of a functional language, a sandbox would either render it useless, or not add much useful protection.
Instead, NixOps4 relies on you as a resource developer to follow some rules.

## Resource Interface

NixOps4 specifies the interface that resources must implement.
Briefly, a resource adheres to the following requirements:
- It is buildable with Nix
- It provides a program <!-- details TBD -->
- It speaks a protocol over stdin/stdout

## Resource Development

Resources can be implemented in any programming language that provides basic I/O facilities.

### Rust

Resources implemented in Rust can use the `nixops4-resource` crate, which takes care of the protocol negotiation.

A resource crate should be named `nixops4-resources-<name>`, where `<name>` is representative of the resource or category of resources implemented.
- `nixops4-resources-*`: plural
  - consistent with the possibility of implementing a _collection_ of resources
  - name for a _categorization_ of resources
- `nixops4-resource-*`: singular
  - for libraries about resources
  - supporting the resource _concept_

