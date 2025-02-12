# Run:
#   nix build .#checks.<system>.nixops4-resources-local
{ hello
, jq
, nixops4-resource-runner
, nixops4-resources-local
, runCommand
, runtimeShell
, writeScriptBin
, die
}:

runCommand
  "check-nixops4-resources-local"
{
  nativeBuildInputs = [
    nixops4-resource-runner
    nixops4-resources-local
    jq
    hello
    die
  ];
}
  ''
    # Test "file" resource

    nixops4-resource-runner create \
      --provider-exe nixops4-resources-local \
      --type file \
      --input-str name test.txt \
      --input-str contents hi \
      > out.json
    cat out.json

    # check that out.json is the empty object
    (set -x; jq -e '. == { }' out.json)

    echo -n hi > expected
    (set -x; diff expected test.txt)

    # Test "exec" resource

    nixops4-resource-runner create \
      --provider-exe nixops4-resources-local \
      --type exec \
      --input-str executable 'hello' \
      --input-json args '["--greeting", "hi there"]' \
      > out.json
    cat out.json

    (set -x; jq -e '. == { "stdout": "hi there\n" }' out.json)

    # Exit code

    (
      set +e
      nixops4-resource-runner create \
        --provider-exe nixops4-resources-local \
        --type exec \
        --input-str executable 'die' \
        --input-json args '["oh no, this and that failed"]' \
        > out.json
      [[ $? == 1 ]]
    )
    cat out.json
    [[ "" == "$(cat out.json)" ]]

    touch $out
  ''
