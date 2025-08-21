# Run:
#   nix build .#checks.<system>.nixops4-resources-local
{ hello
, jq
, nixops4-resource-runner
, nixops4-resources-local
, runCommand
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

    # Test "memo" resource - create with state persistence

    nixops4-resource-runner create \
      --provider-exe nixops4-resources-local \
      --type memo \
      --stateful \
      --input-json initialize_with '"hello world"' \
      > memo_create.json
    cat memo_create.json

    (set -x; jq -e '. == { "value": "hello world" }' memo_create.json)

    # Test "memo" resource - update (should preserve original value)

    nixops4-resource-runner update \
      --provider-exe nixops4-resources-local \
      --type memo \
      --inputs-json '{"initialize_with": "new value"}' \
      --previous-inputs-json '{"initialize_with": "hello world"}' \
      --previous-outputs-json '{"value": "hello world"}' \
      > memo_update.json
    cat memo_update.json

    (set -x; jq -e '. == { "value": "hello world" }' memo_update.json)

    # Test that stateless resources bail on update

    (
      set +e
      nixops4-resource-runner update \
        --provider-exe nixops4-resources-local \
        --type file \
        --inputs-json '{"name": "/tmp/test", "contents": "new"}' \
        --previous-inputs-json '{"name": "/tmp/test", "contents": "old"}' \
        --previous-outputs-json '{}' \
        > file_update_err.json 2>&1
      [[ $? == 1 ]]
    )
    cat file_update_err.json
    grep -F "Internal error: update called on stateless file resource" file_update_err.json

    # Test that memo resource bails when created statelessly

    (
      set +e
      nixops4-resource-runner create \
        --provider-exe nixops4-resources-local \
        --type memo \
        --input-json initialize_with '"test value"' \
        2>&1 | tee memo_stateless_err.json
      [[ $? == 1 ]]
    )
    cat memo_stateless_err.json
    grep -F "memo resources require state (isStateful must be true)" memo_stateless_err.json

    touch $out
  ''
