# NixOps4 Components

<!--
  TODO use mermaid block diagram when out of beta?
  https://mermaid.js.org/syntax/block.html
-->

## Overview

This shows the main types of components that exist around NixOps4.

```mermaid
flowchart TD
  cli["NixOps4"]
  configurations["Configurations"]
  modules["Modules"]
  exprs["Deployment Expressions"]
  resourceProviders["Resource Providers"]
  resources["Resources"]

  configurations -->|are| modules
  modules -->|implement| exprs
  exprs -->|declare| resourceProviders
  resourceProviders -->|operate on| resources
  exprs -->|declare| resources
  cli -->|calls| resourceProviders
  cli -->|calls| exprs
```

## Nix Expressions

```mermaid
flowchart TD
  flakes["Flakes"]
  configurations["Configurations"]
  modules["Modules"]
  resourceProviders["Resource Providers"]

  flakes -->|contain| configurations
  flakes -->|contain| modules
  flakes -->|contain| resourceProviders
  flakes -->|reference by flake inputs| flakes

  configurations -->|reference by imports| modules
  modules -->|reference by imports| modules
  modules -->|reference by flake self or withSystem| resourceProviders
```

Any node can reference packages.

## Crate Structure

NixOps4 is implemented in Rust, and it links to the [Nix package manager](https://nix.dev/manual/nix/latest) to integrate with the Nix language and store.

It is composed of the following Rust crates:

{{#include ./cargo-deps.gen.md}}

