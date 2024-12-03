# Module Options

NixOps4 deployments are composable using the [Module System](https://nixos.org/manual/nixpkgs/stable/#module-system).

Modules loaded into [mkDeployment](../lib/index.md#mkDeployment) can define values for these options, as well as any custom `options` and options provided by other imported modules.

<!-- TODO: some of these options you might not use directly. Link tutorial. -->

<!-- TODO: a lot of options need to be imported and are documented elsewhere. Where to find? -->

## Options

{{#include ./option-docs.gen.md}}
