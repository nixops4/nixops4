# Introduction for users of `nixos-rebuild`

If you already know `nixos-rebuild`, and you are looking for a way to manage your NixOS machines remotely, you **may not need** NixOps.
`nixos-rebuild --target-host` is an option that lets you perform the `switch`, `boot`, and `test` operations on a remote machine over SSH.

Many tools exist that provide similar functionality for groups of machines, that are generally scoped to managing NixOS only.

NixOps4 will additionally manage other types of resources, such as the provisioning of cloud instances, other operating systems, or resources within your NixOS machines.

Note that the implementation is still in progress, so we have to cut the comparison here for now.
<!-- TODO: UI differences, migration -->
