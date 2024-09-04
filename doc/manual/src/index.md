
# Introduction

NixOps4 is a declarative and extensible tool for deploying and managing various types of machines and services.

It integrates tightly with the [Nix package manager](https://nix.dev) to
  - make declarative deployments natural to specify, even for complex setups where resource inputs depend on other resources
  - leverage Nix's package management capabilities to make NixOps itself extensible
  - go beyond just NixOS deployments

This document describes how to use NixOps4, including how to write custom resources for it. Refer to the other projects' documentation to learn how to use NixOps4 with them.
