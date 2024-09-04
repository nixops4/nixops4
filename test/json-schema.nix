{ runCommand, jsonschema }:

runCommand "check-schemas"
{
  nativeBuildInputs = [ jsonschema ];
}
  ''
    (
      set -x;
      jv http://json-schema.org/draft-04/schema# ${../rust/nixops4-resource/resource-schema-v0.json}
      jv ${../rust/nixops4-resource/resource-schema-v0.json}#/definitions/CreateResourceRequest ${../rust/nixops4-resource/examples/v0/CreateResourceRequest.json}
      jv ${../rust/nixops4-resource/resource-schema-v0.json}#/definitions/CreateResourceResponse ${../rust/nixops4-resource/examples/v0/CreateResourceResponse.json}
    )
    touch $out
  ''
