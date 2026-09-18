# Run:
#   nix build .#checks.<system>.nixops4-resource-runner
{
  jq,
  nixops4-resource-runner,
  nixops4-resources-fixture,
  runCommand,
}:

runCommand "check-nixops4-resource-runner"
  {
    nativeBuildInputs = [
      nixops4-resource-runner
      nixops4-resources-fixture
      jq
    ];
  }
  ''
    # A provider that keeps private state has nowhere to put it when the
    # create is stateless, so the protocol asks it to refuse rather than
    # carry on. Omitting --stateful is what a `requireState = true` type
    # must reject.

    (
      set +e
      nixops4-resource-runner create \
        --provider-exe nixops4-resources-fixture \
        --type private_state_echo \
        --input-json new_private_state '"REFUSED"' \
        > stateless-create.stdout 2> stateless-create.log
      echo $? > stateless-create.status
    )
    cat stateless-create.log

    (set -x; [[ "$(cat stateless-create.status)" != 0 ]])
    grep -F "private_state_echo requires state (isStateful must be true)" stateless-create.log

    # A state resource may hold private state of its own, so every
    # operation that carries a resource has to be able to carry it too.

    nixops4-resource-runner create \
      --provider-exe nixops4-resources-fixture \
      --type stateful_state_file \
      --stateful \
      --input-str name state.json \
      --input-json new_private_state '"PRIVATE_V1"' \
      > create.json
    cat create.json

    (set -x; jq -e '. == { "outputProperties": { }, "privateState": "PRIVATE_V1" }' create.json)

    [[ -f state.json ]]
    # create materialises the empty-state skeleton so a later state_read has
    # something to read even before the first event.
    jq -e '. == {"_type": "nixopsState", "resources": {}, "deployments": {}}' state.json

    # The fixture asserts on every operation that it was handed the private
    # state it wrote, so these calls fail if the value is dropped.

    nixops4-resource-runner state-event \
      --provider-exe nixops4-resources-fixture \
      --type stateful_state_file \
      --inputs-json '{"name": "state.json", "expect_own_private_state": "PRIVATE_V1"}' \
      --outputs-json '{}' \
      --private-state-json '"PRIVATE_V1"' \
      --event "create" \
      --nixops-version "4.0.0-test" \
      --patch-json '[
        {
          "op": "add",
          "path": "/resources/myfile",
          "value": {
            "type": "file",
            "inputProperties": {"name": "test.txt"},
            "outputProperties": {}
          }
        }
      ]'

    nixops4-resource-runner state-read \
      --provider-exe nixops4-resources-fixture \
      --type stateful_state_file \
      --inputs-json '{"name": "state.json", "expect_own_private_state": "PRIVATE_V1"}' \
      --outputs-json '{}' \
      --private-state-json '"PRIVATE_V1"' \
      > read.json
    cat read.json

    (set -x; jq -e '.resources.myfile.type == "file"' read.json)

    # Omitting the flag means the provider is handed no private state,
    # which is what a provider that keeps none sees.

    nixops4-resource-runner state-read \
      --provider-exe nixops4-resources-fixture \
      --type stateful_state_file \
      --inputs-json '{"name": "state.json"}' \
      --outputs-json '{}' \
      > read-without.json

    (set -x; jq -e '.resources.myfile.type == "file"' read-without.json)

    # Update is the one operation that both receives and replaces private
    # state, so it is checked against `private_state_echo`, whose whole
    # purpose is to hand back what it was given. The provider fails the
    # call if `expected_prior` is not what the runner sent.

    nixops4-resource-runner create \
      --provider-exe nixops4-resources-fixture \
      --type private_state_echo \
      --stateful \
      --input-json new_private_state '"ECHO_V1"' \
      > echo-create.json
    cat echo-create.json

    (set -x; jq -e '. == { "outputProperties": { }, "privateState": "ECHO_V1" }' echo-create.json)

    nixops4-resource-runner update \
      --provider-exe nixops4-resources-fixture \
      --type private_state_echo \
      --inputs-json '{"new_private_state": "ECHO_V2", "expected_prior": "ECHO_V1"}' \
      --previous-inputs-json '{"new_private_state": "ECHO_V1"}' \
      --previous-outputs-json '{}' \
      --previous-private-state-json '"ECHO_V1"' \
      > echo-update.json
    cat echo-update.json

    (set -x; jq -e '. == { "outputProperties": { }, "privateState": "ECHO_V2" }' echo-update.json)

    # Destroy is part of the provider interface, so it is reachable here
    # too, and carries the private state like the other operations. The
    # fixture removes the state document it was created with.

    nixops4-resource-runner destroy \
      --provider-exe nixops4-resources-fixture \
      --type stateful_state_file \
      --inputs-json '{"name": "state.json", "expect_own_private_state": "PRIVATE_V1"}' \
      --outputs-json '{}' \
      --private-state-json '"PRIVATE_V1"'

    [[ ! -e state.json ]]

    touch $out
  ''
