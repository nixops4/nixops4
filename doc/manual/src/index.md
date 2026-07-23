
# Introduction

NixOps4 is a declarative and extensible tool for deploying and managing various types of machines and services.

It integrates tightly with the [Nix package manager](https://nix.dev) to
  - make declarative deployments natural to specify, even for complex setups where resource inputs depend on other resources
  - leverage Nix's package management capabilities to make NixOps itself extensible
  - go beyond just NixOS deployments

This document describes how to use NixOps4, including how to write custom resources for it. Refer to the other projects' documentation to learn how to use NixOps4 with them.

## Why NixOps4?

### For Terraform / OpenTofu users
<!-- pulumi, CDK -->

**A native Nix-integrated replacement**

NixOps4 fills a role similar to Terraform's, but slightly broader, thanks to its Nix integration.
Benefits:
1. No more awkward bridging between the Nix and Terraform worlds
2. Familiar operating model with a few differences

Differences:
1. State file is first class and can be configured on a per-resource basis
2. More flexible module system
3. Simpler data model
4. Usage philosophy: one model of the world instead of IaC islands
5. In progress: Terraform resource provider compatibility

### For NixOps 1/2 users

**More capable, and sustainable this time**

NixOps 1 was a deployment tool; NixOps 2 added a plugin system.
These versions went unmaintained primarily due to tech debt.

NixOps4 is designed as a platform.
`nixops4` itself is only responsible for the integration and lifecycle of "resources": representations of real-world objects with CRUD operations.

All the details of those resources are implemented in resource providers: interface-bound programs that anyone can build and maintain in any language.

This will give NixOps4 a rich ecosystem of things that can be managed with it, without centralizing the maintenance burden.

### For nixos-rebuild / Colmena / deploy-rs / ... users

**Extend the reach of your config management**

NixOS is the best configuration manager for atomic, whole-host Linux deployments.

NixOps4 extends the familiar NixOS module system to the stateful world outside.
Manage multiple hosts, cloud infrastructure, and application-level configuration such as database setup.
Or build your own open-source cloud on your own hardware, in the same expression that also declares its workloads.

<!--
  Do we pitch to Ansible / Puppet / Chef / Salt users?
  Kubernetes?
  -->
