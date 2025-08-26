# Stateful deployments

## Summary

This document motivates why NixOps 4 supports stateful deployments, and explains why deployment state is optional and first-class.

## Context

NixOps 1 and 2 are stateful deployment tools, in the sense that it maintains a database with information about each deployment.

This has been a point of criticism, because it puts a burden on the user to make sure that the database is used correctly. NixOps 2 has mitigated this by supporting automatic synchronization of the database, but this feature is not enabled by default, and was "bolted on" to the existing application architecture.

Meanwhile, `nixos-rebuild switch --target-host <host>` is a stateless deployment tool. It simply consumes the information passed to it, and does not require any information to be stored and reused between invocations.

However, in cloud deployments, preserving state is often necessary. For instance, when a cloud instance is created, this creates information, such as the SSH public host key, that is necessary to connect to the instance, and may not be stored in the cloud provider's API for us to retrieve later. Even then, retrieving this information may be slow.

Deletions also work differently in stateless vs stateful approaches:
- **Stateless**: Resources must be explicitly deleted by invoking a deletion command before removing them from the deployment expressions
- **Stateful**: Resources can be deleted simply by removing them from the deployment expressions, letting NixOps determine what needs to be deleted based on the stored state

## Decision

The trade-off between stateful and stateless deployments depends on project-specific circumstances.

NixOps 4 can support both, by requiring state to be specified explicitly.

Specifically, a resource can "request" to be connected to a state resource, by requiring a `state` attribute to be filled out.
This naturally requires stateful resources to be part of stateful deployments, but it also allows stateless resources to be part of stateful deployments.

In other words, statefulness is a property of resources and inherited into deployments, but not a property of the deployment tool.
