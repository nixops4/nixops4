# `lib` output attribute of `nixops4` flake
#
# User facing functions for declaring the root component, etc.
#
# Documentation prelude: doc/manual/src/lib/index.md
#
# Tests:
#   ./tests.nix
#
{
  # Nixpkgs lib
  lib,
  # This nixops4 flake
  self,
  # withSystem of the nixops4 flake
  # https://flake.parts/module-arguments#withsystem
  selfWithSystem,
}:

let
  /**
    Evaluate a root component configuration. This is a building block for [`mkRoot`](#mkRoot), which is the usual entry point for defining the root.

    # Type {#evalRoot-type}

    ```
    EvalModulesArguments -> NixOpsArguments -> Configuration
    ```

    # Inputs {#evalRoot-input}

    1. Arguments for [evalModules](https://nixos.org/manual/nixpkgs/stable/#module-system-lib-evalModules) - i.e. the Module System.
       These are adjusted to include NixOps-specific arguments.

    2. Arguments provided by NixOps. These provide the context of the component, including the resource outputs.

    # Output {#evalRoot-output}

    The return value is a [Module System `configuration`](https://nixos.org/manual/nixpkgs/stable/#module-system-lib-evalModules-return-value), including attributes such as `config` and `options`.
  */
  evalRoot =
    baseArgs:
    {
      resourceProviderSystem,
      outputValues,
      ...
    }:
    let
      # Recursively inject output values into resources via lexical scope.
      # Re-declares `members` to add injector modules that capture output
      # values through closures, keeping the internal structure (`outputValues`)
      # private as far as the modules are concerned.
      makeOutputInjector =
        outputsForThis:
        { options, ... }:
        {
          # imports indirection just so we can set _file
          imports = [
            {
              _file = "<resource outputs from nixops>";
              config.outputs = lib.mkIf options.resourceType.isDefined (
                { ... }:
                {
                  config = outputsForThis;
                }
              );
            }
            {
              _file = "<resource output injection support for component members>";
              options.members = lib.mkOption {
                type = lib.types.lazyAttrsOf (
                  lib.types.submoduleWith {
                    modules = [
                      (
                        { name, ... }:
                        {
                          imports = [ (makeOutputInjector (outputsForThis.${name} or { })) ];
                        }
                      )
                    ];
                  }
                );
              };
            }
          ];
        };
    in
    lib.evalModules (
      baseArgs
      // {
        specialArgs = baseArgs.specialArgs // {
          inherit resourceProviderSystem;
        };
        modules = baseArgs.modules ++ [
          (makeOutputInjector outputValues)
        ];
        class = "nixops4Component";
      }
    );

  evalRootForProviders =
    baseArgs:
    { system }:
    evalRoot baseArgs {
      # Input for the provider definitions
      resourceProviderSystem = system;

      # Placeholders that must not be accessed by the provider definitions for pre-building the providers without dynamic resource information
      outputValues = throw "outputValues is not available when evaluating the root for the purpose of building the providers ahead of time.";
    };

in
{
  inherit evalRoot;

  /**
    Turn a list of root component modules and some other parameters into the format expected by the `nixops4` command, and add a few useful attributes.

    # Type {#mkRoot-type}

    ```
    { modules, ... } -> nixops4Component
    ```

    # Input attributes {#mkRoot-input}

    - [`modules`]{#mkRoot-input-modules}: A list of modules to evaluate.

    - [`specialArgs`]{#mkRoot-input-specialArgs}: A set of arguments to pass to the modules these are available while `imports` are evaluated, but are not overridable or extensible, unlike the `_module.args` option.

    - [`prefix`]{#mkRoot-input-prefix}: A list of strings representing the location of the root.
      Typical value: `[ "nixops4" ]`

    # Output attributes {#mkRoot-output}

    - [`_type`]{#mkRoot-output-_type}: `"nixops4Component"`

    - [`rootFunction`]{#mkRoot-output-rootFunction}: Internal value for `nixops4` to use.

    - [`getProviders`]{#mkRoot-output-getProviders}: A function that returns a derivation for the providers of the root.

      [**Input attributes**]{#mkRoot-output-getProviders-input}

      - [`system`]{#mkRoot-output-getProviders-input-system}: The system<!-- TODO: link to docs in https://github.com/NixOS/nixpkgs/pull/324614 when merged --> for which to get the providers.
        Examples:
        - `"x86_64-linux"`
        - `"aarch64-darwin"`

      [**Output**]{#mkRoot-output-getProviders-output}

      A derivation whose output references the providers for the root.

      Note: Currently only collects providers defined at the root level.
      Providers defined in nested components are not included.
  */
  mkRoot =
    {
      modules,
      specialArgs ? { },
      prefix ? [ ],
    }:
    let
      allModules = [ ../component/base-modules.nix ] ++ modules;
      baseArgs = {
        inherit prefix specialArgs;
        modules = allModules;
      };
    in
    {
      _type = "nixops4Component";
      /**
        Internal function for `nixops4` to invoke.
      */
      rootFunction =
        args:
        let
          configuration = evalRoot baseArgs args;
        in
        configuration.config._export;

      # NOTE: not rendered! Update the `mkRoot` docstring above!
      /**
        Get the providers for this root.

        # Input attributes

        - `system`: The system (platform) for which to get the providers.
          Examples:
          - `"x86_64-linux"`
          - `"aarch64-darwin"`

        # Output

        A derivation whose output references the providers for the root.
      */
      getProviders =
        { system }:
        let
          configuration = evalRootForProviders baseArgs { inherit system; };

          # TODO: This only collects root-level providers. Nested components can
          # define their own providers (via providers option), but those are not
          # included here. Fixing this requires user-facing mechanisms to declare
          # which providers should be pre-built, since recursively collecting all
          # providers may not be desirable (e.g., dynamically configured providers).
          serializable = lib.mapAttrs (name: provider: {
            executable = provider.executable;
            args = provider.args;
          }) configuration.config.providers;

        in
        selfWithSystem system (
          { pkgs, ... }:
          (pkgs.writeText "nixops-root-providers" ''
            Store path contents subject to change
            ${builtins.toJSON serializable}
          '').overrideAttrs
            {
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
        and documentation metadata ([`name`][name], [`description`][description], [`sourceBaseUrl`][sourceBaseUrl], [`sourceName`][sourceName])

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
      plus an index.md file.
      The files use mdBook-compatible includes for option documentation.

      [name]: ../modules/index.md#opt-providers._name_.name
      [description]: ../modules/index.md#opt-providers._name_.description
      [sourceBaseUrl]: ../modules/index.md#opt-providers._name_.sourceBaseUrl
      [sourceName]: ../modules/index.md#opt-providers._name_.sourceName
  */
  renderProviderDocs =
    { system, module }:
    selfWithSystem system (
      { config, ... }:
      config.builders.renderProviderDocs {
        inherit module;
      }
    );

}
