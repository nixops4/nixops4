{ lib
, # This nixops4 flake
  self
, # withSystem of the nixops4 flake
  # https://flake.parts/module-arguments#withsystem
  selfWithSystem
,
}:

let
  evalDeployment =
    baseArgs:
    { resources, resourceProviderSystem ? "x86_64-linux" /* FIXME: remove and pass in, in nixops4-eval */, ... }:
    let
      conf =
        lib.evalModules (baseArgs // {
          specialArgs = baseArgs.specialArgs // {
            inherit resources resourceProviderSystem;
          };
        });
    in
    conf // {
      resources =
        lib.mapAttrs
          (name: resource: resource // {
            type = resource.type.type;
          })
          conf.config.resources;
    };

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
  /**
    Turn a list of deployment modules and some other parameters into the format expected by the `nixops4` command, and add a few useful attributes.

    You do not need to call this function directly if you use the `flake-parts` integration.

    # Input attributes

    - `modules`: A list of modules to evaluate.

    - `specialArgs`: A set of arguments to pass to the modules these are available while `imports` are evaluated, but are not overridable or extensible, unlike the `_module.args` option.

    - `prefix`: A list of strings representing the location of the deployment.
      Typical value: `[ "nixops4Deployments" name ]`

    # Output attributes

    - `_type`: `"nixops4Deployment"`

    - `deploymentFunction`: Internal value for `nixops4` to use.

    - `getProviders`: A function that returns the providers for a given system (platform).
      This can be used to build the providers for a deployment ahead of time.

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
          inherit (configuration) resources;
        };

      /**
        Get the providers for this deployment.

        # Input attributes

        - `system`: The system (platform) for which to get the providers.
          Examples:
          - `"x86_64-linux"`
          - `"aarch64-darwin"`

        # Output
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
                  command = provider.command;
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

}
