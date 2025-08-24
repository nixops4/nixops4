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

    # Type {#evalDeployment-type}

    ```
    EvalModulesArguments -> NixOpsArguments -> Configuration
    ```

    # Inputs {#evalDeployment-input}

    1. Arguments for [evalModules](https://nixos.org/manual/nixpkgs/stable/#module-system-lib-evalModules) - i.e. the Module System.
       These are adjusted to include NixOps-specific arguments.

    2. Arguments provided by NixOps. These provide the context of the deployment, including the resource outputs.

    # Output {#evalDeployment-output}

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

  evalDeploymentForProviders =
    baseArgs:
    { system }:
    evalDeployment
      baseArgs
      {
        # Input for the provider definitions
        resourceProviderSystem = system;

        # Placeholders that must not be accessed by the provider definitions for pre-building the providers without dynamic resource information
        resources = throw "resource information is not available when evaluating a deployment for the purpose of building the providers ahead of time.";
      };

in
{
  inherit evalDeployment;

  /**
    Turn a list of deployment modules and some other parameters into the format expected by the `nixops4` command, and add a few useful attributes.

    # Type {#mkDeployment-type}

    ```
    { modules, ... } -> nixops4Deployment
    ```

    # Input attributes {#mkDeployment-input}

    - [`modules`]{#mkDeployment-input-modules}: A list of modules to evaluate.

    - [`specialArgs`]{#mkDeployment-input-specialArgs}: A set of arguments to pass to the modules these are available while `imports` are evaluated, but are not overridable or extensible, unlike the `_module.args` option.

    - [`prefix`]{#mkDeployment-input-prefix}: A list of strings representing the location of the deployment.
      Typical value: `[ "nixops4Deployments" name ]`

    # Output attributes {#mkDeployment-output}

    - [`_type`]{#mkDeployment-output-_type}: `"nixops4Deployment"`

    - [`deploymentFunction`]{#mkDeployment-output-deploymentFunction}: Internal value for `nixops4` to use.

    - [`getProviders`]{#mkDeployment-output-getProviders}: A function that returns a derivation for the providers of the deployment.

      [**Input attributes**]{#mkDeployment-output-getProviders-input}

      - [`system`]{#mkDeployment-output-getProviders-input-system}: The system<!-- TODO: link to docs in https://github.com/NixOS/nixpkgs/pull/324614 when merged --> for which to get the providers.
        Examples:
        - `"x86_64-linux"`
        - `"aarch64-darwin"`

      [**Output**]{#mkDeployment-output-getProviders-output}

      A derivation whose output references the providers for the deployment.
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

      # NOTE: not rendered! Update the `mkDeployment` docstring above!
      /**
        Get the providers for this deployment.

        # Input attributes

        - `system`: The system (platform) for which to get the providers.
          Examples:
          - `"x86_64-linux"`
          - `"aarch64-darwin"`

        # Output

        A derivation whose output references the providers for the deployment.
       */
      getProviders = { system }:
        let
          configuration =
            evalDeploymentForProviders
              baseArgs
              { inherit system; };

          serializable =
            lib.mapAttrs
              (name: provider:
                {
                  executable = provider.executable;
                  args = provider.args;
                }
              )
              configuration.config.providers;

        in
        selfWithSystem system ({ pkgs, ... }:
          (pkgs.writeText
            "nixops-deployment-providers"
            ''
              Store path contents subject to change
              ${builtins.toJSON serializable}
            '').overrideAttrs {
            passthru.config = configuration.config;
          }
        );
    };


  /**
      Generate documentation for a NixOps4 provider module.

      This function renders markdown documentation for all resource types
      defined in a provider module, including their inputs and outputs.

      # Arguments

      - `system`: The system string (e.g., "x86_64-linux")
      - `module`: A NixOps4 provider module containing resource type definitions

      # Example

      ```nix
      renderProviderDocs {
        system = "x86_64-linux";
        module = self.modules.nixops4Provider.local;
      }
      ```

      # Type

      ```
      renderProviderDocs :: {
        system :: String,
        module :: NixModule
      } -> Derivation
      ```

      The resulting derivation contains markdown files for each resource type
      plus an index.md file. The files use mdBook-compatible includes for
      option documentation.
    */
  renderProviderDocs = { system, module }:
    selfWithSystem system ({ config, ... }:
      config.builders.renderProviderDocs {
        inherit module;
      }
    );

}
