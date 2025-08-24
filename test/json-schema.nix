# Run with: nix build .#checks.x86_64-linux.json-schema
{ runCommand
, jsonschema
, json-schema-catalog-rs
, jsonSchemaCatalogs
, jq
,
}:

runCommand "check-schemas"
{
  nativeBuildInputs = [
    jsonschema
    json-schema-catalog-rs
    jsonSchemaCatalogs.json-patch-schemastore
    jq
  ];
}
  ''
    (
      set -x;
      # Validate resource schema (before local schema resolution for JSON Patch references)
      jv http://json-schema.org/draft-04/schema# ${../rust/nixops4-resource/resource-schema-v0.json}

      # Create temporary schema file with resolved references
      json-schema-catalog replace ${../rust/nixops4-resource/resource-schema-v0.json} > resource-schema-v0.json.tmp

      # Validate again (not typically required, but should work)
      jv http://json-schema.org/draft-04/schema# resource-schema-v0.json.tmp

      jv resource-schema-v0.json.tmp#/definitions/CreateResourceRequest ${../rust/nixops4-resource/examples/v0/CreateResourceRequest.json}
      jv resource-schema-v0.json.tmp#/definitions/CreateResourceResponse ${../rust/nixops4-resource/examples/v0/CreateResourceResponse.json}

      # Validate state schema (needs local schema resolution for JSON Patch references)
      jv http://json-schema.org/draft-04/schema# ${../rust/nixops4-resource/state-schema-v0.json}

      # Create temporary schema file with resolved references
      json-schema-catalog replace ${../rust/nixops4-resource/state-schema-v0.json} > state-schema-v0.json.tmp

      jv state-schema-v0.json.tmp#/definitions/State ${../rust/nixops4-resource/examples/state/v0/empty.json}
      jv state-schema-v0.json.tmp#/definitions/StateEvent ${../rust/nixops4-resource/examples/state/v0/initial-event.json}

      # Validate documentation examples
      jq -s '.[0]' ${../doc/manual/src/state/snippets/state-file.json} | jv state-schema-v0.json.tmp#/definitions/StateEvent
      jq -s '.[1]' ${../doc/manual/src/state/snippets/state-file.json} | jv state-schema-v0.json.tmp#/definitions/StateEvent
      jv state-schema-v0.json.tmp#/definitions/State ${../doc/manual/src/state/snippets/resolved-state.json}
    )
    touch $out
  ''
