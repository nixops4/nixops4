# No coupling with NixOS

## Context

NixOps 1 and 2 were tightly coupled with NixOS. This made it hard to use NixOps for other purposes, such as deploying Docker containers, or nix-darwin.

Furthermore, it had a few special files and expectations in the `nixpkgs` repository, which made it hard for NixOS maintainers to understand and maintain, and also made it hard for NixOps to change, as NixOps is expected to support a wide range of NixOS versions.

## Decision

NixOps 4 will not be coupled with NixOS.
It may be mentioned in examples and used in tests, but nothing about the `nixops4` repository will be NixOS-specific.

Instead, the low level, generic interface that is a NixOps resource allows the NixOS integration to be developed in a separate repository, which is loaded into NixOps.

We would like for `nixpkgs` to eventually provide this integration, but it could also be provided by a "third party".
Integration into `nixpkgs` leads to a better user experience, as they are not burdened with matching the NixOS version to the NixOS-NixOps integration version.

It is NixOps' responsibility to make maintaining a resource provider easy, and by having the integration in the same repository, they can make treewide chanegs without having to worry about the integration catching up late.
