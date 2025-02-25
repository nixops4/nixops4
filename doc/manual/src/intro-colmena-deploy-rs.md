# Introduction for users of colmena, deploy-rs

Multiple tools exist that provide functionality for deploying groups of NixOS machines, that are generally scoped to managing NixOS only.

NixOps4 is a more general tool that provides a generic platform for note just NixOS, but also other types of resources, such as the provisioning of cloud instances, other cloud objects such as DNS records, other operating systems, or resources within your NixOS machines.

The architecture of NixOps4 is similar to that of Nix. NixOps4 itself doesn't know how to deploy NixOS or EC2 in the same way that Nix doesn't know how to build a Python package or create a systemd unit.
Unlike its predecessor NixOps 1 (or "2"), or its NixOS-based competition, it is truly a general platform for deployments; not just a tool that has certain deployment behaviors baked in.

Another notable difference is that NixOps4 is capable of "stateful" deployments. That is to say, it can remembers the state of the resources it has deployed, and it uses this info to determine when resources need to be deleted or updated.

Note that the implementation is still in progress, so we have to cut the comparison here for now.
<!-- TODO: UI differences, migration -->

