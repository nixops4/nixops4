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

    # Test "memo" resource

    (
      set -x
      # Create
      in='{ "initialize_with": "123" }'
      nixops4-resource-runner create --provider-exe nixops4-resources-local --type memo --inputs-json "$in" > out.json
      cat out.json
      jq -e '. == { value: "123" }' < out.json
      prev_in="$in"
      prev_out="$(cat out.json)"

      # Update (no-op)
      nixops4-resource-runner update --provider-exe nixops4-resources-local --type memo --inputs-json "$in" --previous-inputs-json "$prev_in" --previous-outputs-json "$prev_out" > out.json
      cat out.json
      jq -e '. == { value: "123" }' < out.json
      prev_in="$in"
      prev_out="$(cat out.json)"

      # Update (ignored)
      in='{ "initialize_with": "456" }'
      nixops4-resource-runner update --provider-exe nixops4-resources-local --type memo --inputs-json "$in" --previous-inputs-json "$prev_in" --previous-outputs-json "$prev_out" > out.json
      cat out.json
      jq -e '. == { value: "123" }' < out.json
      prev_in="$in"
      prev_out="$(cat out.json)"

      # Update (again, ignored)
      # (this time the original *input* is completely lost)
      in='{ "initialize_with": "789" }'
      nixops4-resource-runner update --provider-exe nixops4-resources-local --type memo --inputs-json "$in" --previous-inputs-json "$prev_in" --previous-outputs-json "$prev_out" > out.json
      cat out.json
      jq -e '. == { value: "123" }' < out.json
      prev_in="$in"
      prev_out="$(cat out.json)"
    )

    touch $out
  ''
