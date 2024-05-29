
# Nix for extension

## Context

NixOps 1 was a monolithic Python program, with a single codebase that targeted multiple cloud providers. This was hard to manage, because maintaining a backend required familiarity with the entire system, and if a backend were to go unmaintained, this would block the entire project.

NixOps 2 (pre-release) was a plugin-based continuation of the same program. Although this should have allowed releases to be made more frequently, uncertainty around the project and changes in maintainership have led it to remain in a pre-release state. Furthermore the plugin interface surface area was too large (and, I would add, underdeveloped) for it to become stable.

Nix is a package manager and a configuration language. This makes it suitable for managing the integration of components backed by various tools and languages.

## Decision

NixOps has targeted a wrong level of abstraction. It should be scoped to the basics of managing interrelated resources, without going into domain specifics such as individual cloud providers or even individual use cases of Nix, such as NixOS.

This is an architecture that has been proven by Nix itself in the domain of packaging. NixOps applies the same principles to the domain of declarative resource management (including deployment).

Specifically the interface of resources will be akin to that of Terraform: entities with fields that can be linked up, using the Nix language. The exact interface is to be defined, but it will be simpler than Terraform's.

## Consequences

- NixOps will not come with NixOS support out of the box. This will be a separate project, which is loaded into NixOps.
  This similar to how Nixpkgs is not part of the Nix package manager.

- NixOps will have a well-defined interface for defining resources, which will make it easier for maintainers of backends to understand.

- NixOps has a good chance of producing a stable interface, because it will be scoped to the basics of managing resources, and not the specifics of various resources.

- NixOps will be equally suitable for managing deployments of NixOS as other Nix-based configuration systems.

- There will be no delineation between "tool-provided" and user-provided, therefore not posing an obstacle just as it isn't for packages outside of Nixpkgs, or NixOS modules outside of the `nixpkgs` repository.
