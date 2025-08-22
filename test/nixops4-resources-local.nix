# Run:
#   nix build .#checks.<system>.nixops4-resources-local
{ hello
, jq
, nixops4-resource-runner
, nixops4-resources-local
, runCommand
, jsonschema
, die
,
}:

runCommand "check-nixops4-resources-local"
{
  nativeBuildInputs = [
    nixops4-resource-runner
    nixops4-resources-local
    jq
    hello
    die
    jsonschema
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

    # Test "state_file" resource - create new state file

    nixops4-resource-runner create \
      --provider-exe nixops4-resources-local \
      --type state_file \
      --input-str name "test-state.json" \
      > state_create.json
    cat state_create.json

    (set -x; jq -e '. == { }' state_create.json)

    # Verify the state file was created with initial state

    [[ -f "test-state.json" ]]
    echo "Initial state file contents:"
    cat test-state.json

    # Test state_read - should return empty initial state

    nixops4-resource-runner state-read \
      --provider-exe nixops4-resources-local \
      --type state_file \
      --inputs-json '{"name": "test-state.json"}' \
      --outputs-json '{}' \
      > state_read_initial.json
    cat state_read_initial.json

    (set -x; jq -e '._type == "nixopsState"' state_read_initial.json)
    (set -x; jq -e '.resources == {}' state_read_initial.json)
    (set -x; jq -e '.deployments == {}' state_read_initial.json)

    # Test state_event - add a resource to the state

    nixops4-resource-runner state-event \
      --provider-exe nixops4-resources-local \
      --type state_file \
      --inputs-json '{"name": "test-state.json"}' \
      --outputs-json '{}' \
      --event "create" \
      --nixops-version "4.0.0-test" \
      --patch-json '[
        {
          "op": "add",
          "path": "/resources/myfile",
          "value": {
            "type": "file",
            "inputProperties": {"name": "test.txt", "contents": "hello"},
            "outputProperties": {"inode": 123}
          }
        }
      ]'

    # Test state_read after adding resource

    nixops4-resource-runner state-read \
      --provider-exe nixops4-resources-local \
      --type state_file \
      --inputs-json '{"name": "test-state.json"}' \
      --outputs-json '{}' \
      > state_read_after_add.json
    cat state_read_after_add.json

    (set -x; jq -e '.resources.myfile.type == "file"' state_read_after_add.json)
    (set -x; jq -e '.resources.myfile.inputProperties.name == "test.txt"' state_read_after_add.json)
    (set -x; jq -e '.resources.myfile.inputProperties.contents == "hello"' state_read_after_add.json)
    (set -x; jq -e '.resources.myfile.outputProperties == {"inode": 123}' state_read_after_add.json)

    # Test state_event - modify the resource

    nixops4-resource-runner state-event \
      --provider-exe nixops4-resources-local \
      --type state_file \
      --inputs-json '{"name": "test-state.json"}' \
      --outputs-json '{}' \
      --event "update" \
      --nixops-version "4.0.0-test" \
      --patch-json '[
        {
          "op": "replace",
          "path": "/resources/myfile/inputProperties/contents",
          "value": "modified content"
        }
      ]'

    # Test state_read after modification

    nixops4-resource-runner state-read \
      --provider-exe nixops4-resources-local \
      --type state_file \
      --inputs-json '{"name": "test-state.json"}' \
      --outputs-json '{}' \
      > state_read_after_modify.json
    cat state_read_after_modify.json

    (set -x; jq -e '.resources.myfile.inputProperties.contents == "modified content"' state_read_after_modify.json)
    (set -x; jq -e '.resources.myfile.outputProperties == {"inode": 123}' state_read_after_add.json)

    # Test state_event - remove the resource

    nixops4-resource-runner state-event \
      --provider-exe nixops4-resources-local \
      --type state_file \
      --inputs-json '{"name": "test-state.json"}' \
      --outputs-json '{}' \
      --event "destroy" \
      --nixops-version "4.0.0-test" \
      --patch-json '[
        {
          "op": "remove",
          "path": "/resources/myfile"
        }
      ]'

    # Test state_read after removal

    nixops4-resource-runner state-read \
      --provider-exe nixops4-resources-local \
      --type state_file \
      --inputs-json '{"name": "test-state.json"}' \
      --outputs-json '{}' \
      > state_read_after_remove.json
    cat state_read_after_remove.json

    (set -x; jq -e '.resources == {}' state_read_after_remove.json)

    # Test state_file with existing file - should not modify existing content

    # Make a backup of the existing state file
    cp test-state.json test-state-backup.json

    nixops4-resource-runner create \
      --provider-exe nixops4-resources-local \
      --type state_file \
      --input-str name "test-state.json" \
      > state_create_existing.json
    cat state_create_existing.json

    (set -x; jq -e '. == { }' state_create_existing.json)

    # Verify the existing file was not modified
    (set -x; diff test-state.json test-state-backup.json)

    # Test creating state_file with non-existent file should create it

    nixops4-resource-runner create \
      --provider-exe nixops4-resources-local \
      --type state_file \
      --input-str name "new-state.json" \
      > state_create_new.json
    cat state_create_new.json

    [[ -f "new-state.json" ]]

    # Test invalid state file handling

    echo "invalid json" > invalid-state.json
    (
      set +e
      nixops4-resource-runner create \
        --provider-exe nixops4-resources-local \
        --type state_file \
        --input-str name "invalid-state.json" \
        2>&1 | tee state_invalid_err.json
      [[ $? == 1 ]]
    )

    echo "All state persistence tests passed!"

    # Test documentation examples - verify state-read resolves correctly

    echo "Testing documentation examples state resolution..."

    # Copy the documentation state file example
    cp ${../doc/manual/src/state/snippets/state-file.json} doc-state-file.json

    # Read the state using our state_file resource
    nixops4-resource-runner state-read \
      --provider-exe nixops4-resources-local \
      --type state_file \
      --inputs-json '{"name": "doc-state-file.json"}' \
      --outputs-json '{}' \
      > doc-state-read-result.json

    # Compare with the expected resolved state from documentation
    echo "Expected resolved state:"
    cat ${../doc/manual/src/state/snippets/resolved-state.json}
    echo "Actual state-read result:"
    cat doc-state-read-result.json

    # Check if they match
    jq -e '. == input' doc-state-read-result.json ${../doc/manual/src/state/snippets/resolved-state.json}

    echo "Documentation examples state resolution test passed!"

    touch $out
  ''
