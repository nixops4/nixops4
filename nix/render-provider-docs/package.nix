# Documentation: ../lib/lib.nix
# Tests: ./test/test.nix
{
  lib,
  self,
  providerModule,
  nixosOptionsDoc,
  runCommand,
  writeText,
  ...
}:
let
  # Evaluate the provider module
  providerEval = lib.evalModules {
    modules = [
      ../provider/provider.nix
      { options._module.args = lib.mkOption { internal = true; }; }
      providerModule
    ];
    specialArgs = {
      resourceProviderSystem = throw "Documentation rendering encountered a dependency on `resourceProviderSystem`. This is not allowed, because the docs should be system-independent. Make sure that all built values are covered by `defaultText`. The evaluation trace contains important information about why this was attempted. Often `defaultText` needs to be added to the earliest option in the trace. If it's unclear why, one of the --show-trace items may indicate the fault. Pay special attention to traces from expression files that you control.";
    };
  };

  toCustomMarkdown =
    optionDocs:
    optionDocs.optionsCommonMark.overrideAttrs {
      extraArgs = [
        "--anchor-style"
        "legacy"
      ];
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
  baseUrl = providerEval.config.sourceBaseUrl;
  sourceName = providerEval.config.sourceName;

  transformOption =
    opt:
    opt
    // {
      declarations = lib.concatMap (
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
      ) opt.declarations;
    };

  resourceTypeDocs = lib.mapAttrs evaluateResourceType providerEval.config.resourceTypes;

  # Render individual resource type pages
  renderResourceTypePage =
    name: rt:
    let
      # Check if inputs/outputs have any options
      hasInputs = rt.inputs.optionsNix != { };
      hasOutputs = rt.outputs.optionsNix != { };

      inputsContent = if hasInputs then toCustomMarkdown rt.inputs else null;
      outputsContent = if hasOutputs then toCustomMarkdown rt.outputs else null;
    in
    {
      name = "${name}.md";
      path = runCommand "${name}.md" { } ''
        cat > $out << 'EOF'
        # ${name}

        ${rt.description}

        **State Required:** ${if rt.requireState then "Yes" else "No"}

        ## Inputs

        EOF
        ${if inputsContent != null then "cat ${inputsContent} >> $out" else "echo '_(none)_' >> $out"}
        cat >> $out << 'EOF'

        ## Outputs

        EOF
        ${if outputsContent != null then "cat ${outputsContent} >> $out" else "echo '_(none)_' >> $out"}
      '';
    };

  # Render index page
  indexPage = writeText "index.md" ''
    # ${providerEval.config.name}

    ${providerEval.config.description}

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
  ++ (lib.mapAttrsToList renderResourceTypePage resourceTypeDocs);

  providerDocsDir = runCommand "provider-docs-${sourceName}" { } ''
    mkdir -p $out
    ${lib.concatMapStringsSep "\n" (page: "cp ${page.path} $out/${page.name}") allPages}
  '';

in
providerDocsDir
