# Module Options

NixOps4 deployments are composable using the [Module System](https://nixos.org/manual/nixpkgs/stable/#module-system).

Modules loaded into [mkDeployment](../lib/index.md#mkDeployment) can define values for these options, as well as any custom `options` and options provided by other imported modules.

<div class="warning">

**Pay attention to examples in the parent options**

The NixOps4 modules use a patterns you may not be familiar with as a NixOS user or contributor: `imports` into `submodule`, and the `deferredModule` type.

These enable the "building with bricks" experience instead of a "filling in a form" experience.

This means that the options below only outline the low level interface, whereas often you'll use `imports` and then support other options.

</div>

## Options

{{#include ./option-docs.gen.md}}
