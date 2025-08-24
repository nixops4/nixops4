{ lib
, self
, providerModule
, nixosOptionsDoc
, runCommand
, writeText
, ...
}:
let
  # Evaluate the local provider module
  providerEval = lib.evalModules {
    modules = [
      ../../nix/provider/provider.nix
      { options._module.args = lib.mkOption { internal = true; }; }
      providerModule
    ];
    specialArgs = {
      resourceProviderSystem = throw "Documentation generation encountered a dependency on `resourceProviderSystem`. This is not allowed, because the docs should be system-independent. Make sure that all built values are covered by `defaultText`. The evaluation trace contains important information about why this was attempted. Often `defaultText` needs to be added to the earliest option in the trace. If it's unclear why, one of the --show-trace items may indicate the fault. Pay special attention to traces from expression files that you control."; # Placeholder for docs
    };
  };

  # For each resource type, evaluate its inputs and outputs modules
  evaluateResourceType = name: resourceType: {
    inherit name;
    inherit (resourceType) description requireState;

    inputs = nixosOptionsDoc {
      options =
        (lib.evalModules {
          prefix = [ "inputs" ];
          modules = [ resourceType.inputs ];
        }).options;
      transformOptions = transformOption;
    };

    outputs = nixosOptionsDoc {
      options =
        (lib.evalModules {
          prefix = [ "outputs" ];
          modules = [ resourceType.outputs ];
        }).options;
      transformOptions = transformOption;
    };
  };

  sourcePathStr = "${self.outPath}";
  baseUrl = "https://github.com/nixops4/nixops4/tree/main";
  sourceName = "nixops4";

  transformOption =
    opt:
    opt
    // {
      declarations = lib.concatMap
        (
          decl:
          let
            # Remove ", via ..." suffix that deferredModule adds
            declStr = toString decl;
            cleanDeclStr = lib.head (lib.splitString ", via " declStr);
          in
          if lib.hasPrefix sourcePathStr cleanDeclStr then
            let
              subpath = lib.removePrefix sourcePathStr cleanDeclStr;
            in
            [
              {
                url = baseUrl + subpath;
                name = sourceName + subpath;
              }
            ]
          else
            [ ]
        )
        opt.declarations;
    };

  resourceTypeDocs = lib.mapAttrs evaluateResourceType providerEval.config.resourceTypes;

  # Generate individual resource type pages
  generateResourceTypePage =
    name: rt:
    let
      # Check if inputs/outputs have any options
      hasInputs = rt.inputs.optionsNix != { };
      hasOutputs = rt.outputs.optionsNix != { };

      inputsContent = if hasInputs then "{{#include ${rt.inputs.optionsCommonMark}}}" else "_(none)_";
      outputsContent = if hasOutputs then "{{#include ${rt.outputs.optionsCommonMark}}}" else "_(none)_";
    in
    {
      name = "${name}.md";
      path = writeText "${name}.md" ''
        # ${name}

        ${rt.description}

        **State Required:** ${if rt.requireState then "Yes" else "No"}

        ## Inputs

        ${inputsContent}

        ## Outputs

        ${outputsContent}
      '';
    };

  # Generate index page
  indexPage = writeText "index.md" ''
    # Local Provider

    The local provider implements resources that operate on the local system.

    They are atypical, as most resources represent a single real world entity that is reached over the network, but a resource like `file` is not singular like that, when NixOps4 is invoked from different environments.

    ## Resource Types

    ${lib.concatMapStringsSep "\n" (
      name: "- [${name}](./${name}.md) <!-- TODO: add shortDescription when implemented -->"
    ) (lib.attrNames resourceTypeDocs)}
  '';

  # Create flat directory with all pages as individual files
  allPages = [
    {
      name = "index.md";
      path = indexPage;
    }
  ]
  ++ (lib.mapAttrsToList generateResourceTypePage resourceTypeDocs);

  providerDocsDir = runCommand "local-provider-docs" { } ''
    mkdir -p $out
    ${lib.concatMapStringsSep "\n" (page: "cp ${page.path} $out/${page.name}") allPages}
  '';

in
providerDocsDir
