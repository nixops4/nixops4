
# Nix for deployment configuration

## Summary

A motivation for integrating a deployment tool into Nix.

## Context

The primary purpose of NixOps is to provide a declarative way to manage deployments.

"Declarative" means that the user states their desired end state, without explicitly sequencing actions ("control flow").

This end state can only materialize through a sequence of steps, through some tool that performs the actions necessary.

Unlike imperative systems, where e.g. `execve` is a reasonably good, highly standard interface for "large scale" integration across ecosystems, declarative systems have no such standard interface. I ascribe this to the fact that declarative systems are more complex and diverse, demanding more than what operating systems "naturally" provide. For instance, multiple programming paradigms may be used, and the operations on the declared entities will differ between systems.

It follows that boundaries between declarative systems are a hindrance to integration.

Nix is a configuration language and package manager, both of which are useful tools for managing deployments.

## Decision

NixOps will integrate deeply with Nix, by using Nix as its configuration language.

## Consequences

- NixOps will be able to manage deployments in a way that is consistent with the rest of the Nix ecosystem.

- NixOps will be able to leverage the Nix package manager to manage the software running on the deployments.

- NixOps deployments can reuse the Nixpkgs library, which provides many useful tools such as
  - image generation
  - configuration file generation
  - NixOS
  - a configuration DSL ("module system") to make the language more user friendly, robust and composable
