{ runCommand, jsonschema }:

runCommand "check-schemas"
{
  nativeBuildInputs = [ jsonschema ];
}
  ''
    (
      set -x;
      jv http://json-schema.org/draft-04/schema# ${../rust/nixops4-resource/resource-schema-v0.json}
    )
    touch $out
  ''
