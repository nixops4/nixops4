 # Message architecture

## Status: DRAFT

This document outlines the conceptual approach to interprocess communication and deployment coordination in NixOps 4.

**Important**: The examples and implementation details in this document are illustrative and may not reflect the exact current implementation. The core architectural principles remain valid, but specific interfaces, message formats, and flake structures may have evolved.

## Context

Considering the previously outlined architecture, we need to define the messages that will be passed between the components.

For this purpose, we'll look into the process of deploying an application that uses a single cloud resources.

We'll assume that a deployment expression looks somewhat as follows, ignoring the possibility to use Nix to create conveniences and abstractions, and also ignoring proper flake authoring.

**Note**: This example is conceptual and does not represent the current API. It serves to illustrate the message flow patterns rather than the exact implementation.

In practice, almost all boilerplate will be reduced to a `flake-parts` module and/or helper functions.

<details><summary>Conceptual `flake.nix` example (not current API)</summary>

```nix
{
  inputs.nixops4.url = "github:nixops4/nixops4";
  inputs.nixops4-aws.url = "github:nixops4/nixops4-aws";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { self, nixops4 }:
    let
      # simplification to reduce document complexity: in practice, the deployer and deployed platforms may differ
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};

    in
    {
      apps.${system}.nixops.command = nixops4 + "/bin/nixops4";

      # This is not the final user interface, but the return value of something like
      # nixopsProjects.default = nixops4.lib.mkProject { } ./deployment.nix;
      nixopsProjects.default = { __mkResource, resources, ... }: {
        
        resources = {
          # __mkResource would not be called by the user, but by an expression in `inputs.nixops4-aws.lib`, which passes the `properties` along.
          bucket = __mkResource {
            provider = inputs.nixops4-aws.packages.${system}.nixops4-provider;
            args = ["s3-bucket"];
            properties = {
              auth = {
                # ...
              };
              name = "my-bucket";
              region = "us-west-1";
            };
          };

          instance = __mkResource {
            provider = inputs.nixops4-aws.packages.${system}.nixops4-provider;
            args = ["ec2-instance"];
            properties = {
              auth = {
                # ...
              };
              region = "us-west-1";
              instanceType = "t3.micro";
              # ...
            };
          };

          nixos = __mkResource {
            provider = inputs.nixops4.packages.${system}.nixops4-local-command;
            properties = {
              command = sshWrapper { # akin to https://docs.hercules-ci.com/hercules-ci-effects/reference/nix-functions/ssh
                destination = "root@${resources.instance.ip}";
                command = pkgs.writeScript "deploy-nixos" ''
                  ${(nixpkgs.lib.nixosSystem { modules = [
                      { services.myapp.settings.bucket = resources.bucket.url; }
                    ]; }).config.system.toplevel}/bin/apply
                '';
              };
            };
          };

          dns = __mkResource {
            provider = inputs.nixops4.packages.${system}.nixops4-aws;
            args = ["route53-record"];
            properties = {
              auth = {
                # ...
              };
              zone = "example.com";
              name = "myapp.example.com";
              type = "A";
              ttl = 300;
              records = [resources.instance.ip];
            };
          };
        };
      };
    };
}
```

</details>

## Decision

The following describes the conceptual message flow when a user initiates deployment. The specific command syntax and implementation details may differ in practice.

1. Nix builds NixOps4 and runs it.
2. `nixops4` executable starts its subcommand `deploy`
3. `deploy` loads the `nixopsProjects.default` expression
  - Create the evaluator process
  - Tell the evaluator process to load the `default` project, causing it to evaluate that attribute
4. `deploy` queries the evaluator for the resources
  - The evaluator invokes the `nixopsProjects.default` function, passing it the internally defined `__mkResource` function (a Nix "primop"), and a `resources` attribute set containing thunks that refer to unnamed primops.
    - Note: The values in the `resources` attribute set are not representable as pure Nix values. They require execution of Rust code to produce the Nix primop values that will be accessible later, as demonstrated in the step-by-step example below.
  - The evaluator then forces the `resources` attribute set for its names only.
  - `deploy` receives `["bucket", "dns", "instance", "nixos"]` in the example above. Note that attributes are semantically unordered and returned alphabetically.
5. `deploy` enters an event loop in which it tries to get as much information about the resources as possible, creates processes for the resources, waits for them to finish and reports the results to evaluator and to the user. This involves lazily evaluating resources, where some values require spawning I/O operations and interacting with the external world to resolve. When dependencies are unresolved, the system can backtrack and retry evaluation after the required information becomes available. We'll describe it using the example project, and in a more linear fashion than a typical execution would be. Where it says "Section", it's a grouping to help with reading, rather than actions that translate into actual code.
  - Section: Step 1
  - `deploy` queries each resource in no particular order, but perhaps in the order they were returned by the evaluator.


  - Section: Visiting `bucket`
    - `deploy` queries the `bucket` resource
      - The evaluator forces the `provider`, `args` and `properties`, to the extent possible
      - In this case, the evaluator simply finds the information.
      - The evaluator returns the gathered information to the `deploy` process.
    - `deploy` sees that the `bucket` resource is ready to run, realises the `provider`, and starts it with `args` (`s3-bucket`)
    - While the provider is running, `nixops4` will continue on other resources, but for the purpose of explaining the process, we'll ignore that for now.
    - `deploy` sends the `properties` to the `nixops4-aws s3-bucket` process
    - `nixops4-aws s3-bucket` uses the provided `auth` field and ambient environment to authenticate agains the AWS API, and creates the bucket.
    - `nixops4-aws s3-bucket` reports the result to the `deploy` process
    - `deploy` reports the resulting properties to the evaluator.

  - Section: Continuing to the next resource, `dns`
    - `deploy` queries the `dns` resource
      - The evaluator forces the `provider`, `args` and `properties`, to the extent possible
      - An exception occurs in `properties.records[0]`, because `resources.instance.ip` is not yet known.
      - The evaluator reports both attribute paths
    - `deploy` sees that the `dns` resource is not ready to run, and adds it to a list of actions to retry after `instance.ip` is ready. (ie in a `map<attrpath, list<attrpath>>`)

  - At this point it seems best to go depth first - ie query the `instance` resource next, although a breadth first approach is also possible.
    In fact, since we are usually network-bound, a concurrent implementation is preferable, and as long as the evaluator outpaces the network, it will re-evaluate "inefficiently" anyway.

  - Section: Continuing to the next resource, `instance`
    - This resource is ready to run; see the `bucket` resource for how it's done.
  
  - Section: Revisiting `dns`
    - During the `instance` resource's execution, the `instance.ip` attribute was set.
      This triggers the `dns` resource to be evaluated again.
    - The Nix language does not remember exceptions (and if it does, it should allow specific exceptions to avoid the remembering behavior), so that now all inputs to the `dns` resource evaluate.
    - The `dns` resource is ready to run; see the `bucket` resource for how it's done.

  - Section: Continuing to the next resource, `nixos`
    - This resource is ready to run; see the `bucket` resource for how it's done.
    - Note that one of the inputs is a NixOS evaluation, which is performed by the evaluator process.
      All resource inputs are evaluated and realised.

      - Alternative: A resource could implement a separate evaluator process, whose inputs are file based.
        This would allow the advantage of caching the NixOS evaluation, at the cost of slightly reduced flexibility.

6. Wrap-up. Now that all evaluation and deployment is done, the evaluator process is told to stop, and the `deploy` process reports the final results to the user.

## Implementation Notes

The core architectural principles of process separation, lazy evaluation, and dependency-driven resource coordination described in this document remain central to the current implementation. The main command is `nixops4 apply` rather than `nixops4 deploy`, and specific API details may vary, but the overall message flow and coordination patterns are accurately represented.
