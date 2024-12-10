# `lib` output attribute of `nixops4` flake
#
# User facing functions for declaring deployments, etc.
#
# Documentation prelude: doc/manual/src/lib/index.md
#
# Tests:
#   ./tests.nix
#
{
  # Nixpkgs lib
  lib
, # This nixops4 flake
  self
, # withSystem of the nixops4 flake
  # https://flake.parts/module-arguments#withsystem
  selfWithSystem
,
}:

let
  /**
    Evaluate a deployment configuration. This is a building block for [`mkDeployment`](#mkDeployment), which is the usual entry point for defining deployments.

    # Type

    ```
    EvalModulesArguments -> NixOpsArguments -> Configuration
    ```

    # Inputs

    1. Arguments for [evalModules](https://nixos.org/manual/nixpkgs/stable/#module-system-lib-evalModules) - i.e. the Module System.
       These are adjusted to include NixOps-specific arguments.

    2. Arguments provided by NixOps. These provide the context of the deployment, including the resource outputs.

    # Output

    The return value is a [Module System `configuration`](https://nixos.org/manual/nixpkgs/stable/#module-system-lib-evalModules-return-value), including attributes such as `config` and `options`.
  */
  evalDeployment =
    baseArgs:
    { resources, resourceProviderSystem, ... }:
    lib.evalModules (baseArgs // {
      specialArgs = baseArgs.specialArgs // {
        inherit resources resourceProviderSystem;
      };
      class = "nixops4Deployment";
    });

in
{
  inherit evalDeployment;

  /**
    Turn a list of deployment modules and some other parameters into the format expected by the `nixops4` command, and add a few useful attributes.

    # Input attributes

    - `modules`: A list of modules to evaluate.

    - `specialArgs`: A set of arguments to pass to the modules these are available while `imports` are evaluated, but are not overridable or extensible, unlike the `_module.args` option.

    - `prefix`: A list of strings representing the location of the deployment.
      Typical value: `[ "nixops4Deployments" name ]`

    # Output attributes

    - `_type`: `"nixops4Deployment"`

    - `deploymentFunction`: Internal value for `nixops4` to use.

   */
  mkDeployment = { modules, specialArgs ? { }, prefix ? [ ] }:
    let
      allModules = [ ../deployment/base-modules.nix ] ++ modules;
      baseArgs = {
        inherit prefix specialArgs;
        modules = allModules;
      };
    in
    {
      _type = "nixops4Deployment";
      /**
        Internal function for `nixops4` to invoke.
       */
      deploymentFunction =
        args:
        let
          configuration =
            evalDeployment
              baseArgs
              args;
        in
        {
          resources = lib.mapAttrs (_: res: res._resourceForNixOps) configuration.config.resources;
        };
    };

}
