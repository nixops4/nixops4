# Run with: nix build .#checks.x86_64-linux.json-schema
{ runCommand, jsonschema, json-schema-catalog-rs, jsonSchemaCatalogs }:

runCommand "check-schemas"
{
  nativeBuildInputs = [ jsonschema json-schema-catalog-rs jsonSchemaCatalogs.json-patch-schemastore ];
}
  ''
    (
      set -x;
      # Validate resource schema
      jv http://json-schema.org/draft-04/schema# ${../rust/nixops4-resource/resource-schema-v0.json}
      jv ${../rust/nixops4-resource/resource-schema-v0.json}#/definitions/CreateResourceRequest ${../rust/nixops4-resource/examples/v0/CreateResourceRequest.json}
      jv ${../rust/nixops4-resource/resource-schema-v0.json}#/definitions/CreateResourceResponse ${../rust/nixops4-resource/examples/v0/CreateResourceResponse.json}

      # Validate state schema (needs local schema resolution for JSON Patch references)
      jv http://json-schema.org/draft-04/schema# ${../rust/nixops4-resource/state-schema-v0.json}

      # Create temporary schema file with resolved references
      json-schema-catalog replace ${../rust/nixops4-resource/state-schema-v0.json} > state-schema-v0.json.tmp

      jv state-schema-v0.json.tmp#/definitions/State ${../rust/nixops4-resource/examples/state/v0/empty.json}
      jv state-schema-v0.json.tmp#/definitions/StateEvent ${../rust/nixops4-resource/examples/state/v0/initial-event.json}
    )
    touch $out
  ''
