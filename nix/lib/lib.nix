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
  lib,
  # This nixops4 flake
  self,
  # withSystem of the nixops4 flake
  # https://flake.parts/module-arguments#withsystem
  selfWithSystem,
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
    {
      resources,
      resourceProviderSystem,
      deployments,
      ...
    }@args:
    lib.evalModules (
      baseArgs
      // {
        specialArgs = baseArgs.specialArgs // {
          inherit resources resourceProviderSystem deployments;
        };
        class = "nixops4Deployment";
      }
    );

  evalDeploymentForProviders =
    baseArgs:
    { system }:
    evalDeployment baseArgs {
      # Input for the provider definitions
      resourceProviderSystem = system;

      # Placeholders that must not be accessed by the provider definitions for pre-building the providers without dynamic resource information
      resources = throw "resource information is not available when evaluating a deployment for the purpose of building the providers ahead of time.";

      # Placeholders that must not be accessed by the provider definitions for pre-building the providers without dynamic resource information
      # FIXME: this should probably just be implemented, TBD
      deployments = throw "sub-deployment information is not available when evaluating a deployment for the purpose of building the providers ahead of time.";
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
  mkDeployment =
    {
      modules,
      specialArgs ? { },
      prefix ? [ ],
    }:
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
          configuration = evalDeployment baseArgs args;
        in
        configuration.config._export;

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
      getProviders =
        { system }:
        let
          configuration = evalDeploymentForProviders baseArgs { inherit system; };

          serializable = lib.mapAttrs (name: provider: {
            executable = provider.executable;
            args = provider.args;
          }) configuration.config.providers;

        in
        selfWithSystem system (
          { pkgs, ... }:
          (pkgs.writeText "nixops-deployment-providers" ''
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

  /**
    Turn a Terraform provider package into a NixOps4 provider module.

    # Type

    ```
    terraformSchemaToModule :: AttrSet -> String -> AttrSet
    ```

    # Arguments

    - `terraformProvider`: The terraform provider

    # Returns

    A module to import into one of your deployment's providers.<name> submodules.
  */
  tfProviderToModule =
    { tfProvider }:
    let
      schemaDerivation = selfWithSystem tfProvider.stdenv.hostPlatform.system (
        { config, ... }: config.builders.generateCommonTfSchema { inherit tfProvider; }
      );

      schemaJSON = builtins.trace "SCHEMA: ${schemaDerivation}/schema.json" builtins.fromJSON (
        builtins.readFile "${schemaDerivation}/schema.json"
      );
    in
    self.lib.tfCommonSchemaToModule tfProvider schemaJSON;

  # TODO: Implement the actual translation from schemaJson to NixOps4 module
  # Extract terraform provider executable path
  providerPath =
    pkg:
    "${pkg}/libexec/terraform-providers/${pkg.provider-source-address}/${pkg.version}/${pkg.GOOS}_${pkg.GOARCH}/${pkg.pname}_${pkg.version}";

  tfCommonSchemaToModule =
    pkg: schema:
    let

      # Convert Terraform attribute type to NixOS module option type
      terraformTypeToOptionType =
        tfType:
        if tfType == "\"string\"" then
          lib.types.str
        else if tfType == "\"bool\"" then
          lib.types.bool
        else if tfType == "\"number\"" then
          lib.types.int
        else
          lib.types.unspecified; # fallback - leave complex types unimplemented for now

      # Convert a Terraform block (attributes + block_types) to NixOS module options
      # Parameters:
      # - block: The Terraform block containing attributes and block_types
      # - nameTransform: Function to transform option names (name -> newName)
      # - attributeFilter: Function to filter attributes (attr -> bool)
      # - blockTypeFilter: Function to filter block types (blockType -> bool)
      # - descriptionTemplate: Function to generate descriptions (name -> type -> description)
      convertBlock =
        {
          block,
          nameTransform ? (name: name),
          attributeFilter ? (_: true),
          blockTypeFilter ? (_: true),
          descriptionTemplate ? (name: type: "${type} ${name}"),
        }:
        let
          # Convert simple attributes
          attributeOptions = lib.concatMapAttrs (
            name: attr:
            lib.optionalAttrs (attributeFilter attr) {
              ${nameTransform name} = lib.mkOption {
                type = terraformTypeToOptionType attr.type;
                description =
                  if attr.description != null && attr.description != "" then
                    attr.description
                  else
                    descriptionTemplate name "attribute";
                # TODO: Handle defaults and required attributes properly
              };
            }
          ) (block.attributes or { });

          # Convert block types to appropriate option types based on nesting mode
          blockTypeOptions = lib.concatMapAttrs (
            name: blockType:
            lib.optionalAttrs (blockTypeFilter blockType) {
              ${nameTransform name} = lib.mkOption {
                type =
                  if blockType.nesting == "Single" then
                    lib.types.nullOr (
                      lib.types.submodule {
                        options = lib.mapAttrs (
                          attrName: attr:
                          lib.mkOption {
                            type = terraformTypeToOptionType attr.type;
                            description =
                              if attr.description != null && attr.description != "" then
                                attr.description
                              else
                                "Block attribute ${attrName}";
                            default = if attr.optional then null else lib.mkDefault null;
                          }
                        ) (blockType.block.attributes or { });
                      }
                    )
                  else
                    lib.types.listOf (
                      lib.types.submodule {
                        options = lib.mapAttrs (
                          attrName: attr:
                          lib.mkOption {
                            type = terraformTypeToOptionType attr.type;
                            description =
                              if attr.description != null && attr.description != "" then
                                attr.description
                              else
                                "Block attribute ${attrName}";
                            default = if attr.optional then null else lib.mkDefault null;
                          }
                        ) (blockType.block.attributes or { });
                      }
                    );
                description =
                  if blockType.block.description != null && blockType.block.description != "" then
                    blockType.block.description
                  else
                    descriptionTemplate name "block";
                default = if blockType.nesting == "Single" then null else [ ];
              };
            }
          ) (block.block_types or { });

        in
        attributeOptions // blockTypeOptions;

      # Convert Terraform schema attributes to NixOS module input options
      convertInputAttributes =
        attrs:
        convertBlock {
          block = {
            attributes = attrs;
          };
          attributeFilter =
            attr:
            # Filter to include only attributes that can/should be provided by users:
            # - Required attributes: MUST be provided by user (required: true)
            # - Optional attributes: CAN be provided by user (optional: true)
            # - Optional+Computed attributes: CAN be provided by user OR computed by provider
            # - Computed-only attributes: Should NOT appear in inputs (only in outputs)
            # This covers: required-only, optional-only, and optional+computed cases
            attr.optional || attr.required;
          descriptionTemplate = name: type: "Terraform input ${name}";
        };

      # Convert Terraform schema attributes to NixOS module output options
      convertOutputAttributes =
        attrs:
        lib.mapAttrs
          (
            name: attr:
            lib.mkOption {
              type = terraformTypeToOptionType attr.type;
              description =
                if attr.description != null && attr.description != "" then
                  attr.description
                else
                  "Terraform output ${name}";
              readOnly = true;
            }
          )
          (
            # Filter to include only attributes that are computed/set by the provider:
            # - Computed-only attributes: Set by provider, not user configurable
            # - Optional+Computed attributes: Can be set by provider if not provided by user
            # This covers: computed-only and optional+computed cases
            lib.filterAttrs (_: attr: attr.computed) attrs
          );

      # Convert provider-level configuration to inputs with tf-provider- prefix
      convertProviderConfiguration =
        providerBlock:
        convertBlock {
          block = providerBlock;
          nameTransform = name: "tf-provider-${name}";
          attributeFilter =
            attr:
            # Apply same filtering logic as inputs: include user-configurable attributes
            attr.optional || attr.required;
          blockTypeFilter = _: true; # Include all block types for provider config
          descriptionTemplate = name: type: "Terraform provider ${type} ${name}";
        };

      convertProvider =

        typePrefix: name: value:
        # https://nixops.dev/manual/development/resource-provider/index.html?highlight=requireState#state-requirements
        {
          options = {
            # TODO: (nice to have for later) It would be nice to generate these at top level, for a more tf-like experience.
            #       Tricky: some properties are both inputs and outputs.
            #         - outputs must be forwarded to the option value without destroying definitions. `apply` lets us do that.
            #         - definitions must be forwarded to `inputs`. Can we evaluate the definitions without apply?
          };

          config = {
            # https://flake.parts/options/nixops4.html#opt-nixops4Deployments._name_.providers._name_.resourceTypes._name_.provider.executable
            provider.executable = lib.getExe (
              selfWithSystem pkg.stdenv.hostPlatform.system (
                { config, ... }: config.packages.nixops4-resources-terraform-release
              )
            );
            # https://flake.parts/options/nixops4.html#opt-nixops4Deployments._name_.providers._name_.resourceTypes._name_.provider.args
            provider.args = [
              "run"
              "--provider-exe"
              (self.lib.providerPath pkg)
            ];
            # https://flake.parts/options/nixops4.html#opt-nixops4Deployments._name_.providers._name_.resourceTypes._name_.provider.type
            provider.type = typePrefix + name;

            # https://flake.parts/options/nixops4.html#opt-nixops4Deployments._name_.providers._name_.resourceTypes._name_.requireState
            requireState = true; # TODO: study ephemeral resources

            # Placeholder description from schema
            description = fallBackStr (value.block.description or null) "Terraform resource ${name}";

            # Set which input names are optional based on Terraform schema (including provider attributes and block types)
            isOptionalInputName =
              let
                # Combine resource attributes with provider attributes (with tf-provider- prefix)
                allAttrs =
                  value.block.attributes
                  // lib.mapAttrs' (
                    name: attr: lib.nameValuePair "tf-provider-${name}" attr
                  ) schema.provider.block.attributes
                  # Add provider block types as optional (they default to empty lists)
                  // lib.mapAttrs' (
                    name: blockType:
                    lib.nameValuePair "tf-provider-${name}" {
                      optional = true;
                      required = false;
                    }
                  ) (schema.provider.block.block_types or { });
              in
              inputName:
              let
                attr = allAttrs.${inputName} or null;
              in
              attr != null && attr.optional && !attr.required;

            # https://flake.parts/options/nixops4.html#opt-nixops4Deployments._name_.providers._name_.resourceTypes._name_.inputs
            inputs = {
              options =
                convertBlock {
                  block = value.block;
                  attributeFilter = attr: attr.optional || attr.required;
                }
                // convertProviderConfiguration schema.provider.block;
            };

            # https://flake.parts/options/nixops4.html#opt-nixops4Deployments._name_.providers._name_.resourceTypes._name_.outputs
            outputs = {
              options = convertOutputAttributes value.block.attributes;
            };
          };
        };
      fallBackStr = v: def: if v == null || v == "" then def else v;
    in
    { lib, ... }:
    # https://nixops.dev/manual/development/resource-provider/index.html?highlight=requireState#resource-type-declaration
    # https://flake.parts/options/nixops4.html#opt-nixops4Deployments._name_.providers
    {
      name = lib.getName pkg;

      description = fallBackStr (schema.provider.block.description or "") (
        fallBackStr (pkg.meta.description or "") (lib.getName pkg)
      );

      sourceName = pkg.pname or (lib.getName pkg);
      sourceBaseUrl =
        pkg.meta.homepage or "https://registry.terraform.io/providers/${pkg.provider-source-address}";

      resourceTypes =
        lib.concatMapAttrs (name: value: {
          ${name} = convertProvider "" name value;
        }) schema.resource_schemas
        // lib.concatMapAttrs (name: value: {
          "data-source-${name}" = convertProvider "get-" name value;
        }) schema.data_source_schemas;
    };

}
