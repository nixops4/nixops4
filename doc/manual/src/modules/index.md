# Module Options

NixOps4 deployments are composable using the [Module System](https://nixos.org/manual/nixpkgs/stable/#module-system).

Modules loaded into [mkDeployment](../lib/index.md#mkDeployment) can define values for these options, as well as any custom `options` and options provided by other imported modules.

<div class="warning">

**Pay attention to examples in the parent options**

The NixOps4 modules use a patterns you may not be familiar with as a NixOS user or contributor: `imports` into `submodule`, and the `deferredModule` type.

These enable the "building with bricks" experience instead of a "filling in a form" experience.

Crucially, this means that using the option paths below as a template may lead you down the wrong path. By looking at the parent options first, you will find concise examples what use other modules to define the values of whole groups of suboptions.

<!-- FIXME: This is most likely a significant problem. Just generate separate pages. -->

</div>

<!-- TODO: some of these options you might not use directly. Link tutorial. -->

## Options

{{#include ./option-docs.gen.md}}
