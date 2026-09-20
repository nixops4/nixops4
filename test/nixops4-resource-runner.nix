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
    nixops4-resource-runner create \
      --provider-exe nixops4-resources-fixture \
      --type stateful_state_file \
      --stateful \
      --input-str name state.json \
      > create.json
    cat create.json

    # stateful_state_file declares no outputs, so the response carries none.
    jq -e '. == {}' create.json

    [[ -f state.json ]]
    # create materialises the empty-state skeleton so a later state_read has
    # something to read even before the first event.
    jq -e '. == {"_type": "nixopsState", "resources": {}, "deployments": {}}' state.json

    # Destroy is part of the provider interface, so it is reachable here
    # too. The fixture removes the state document it was created with.
    nixops4-resource-runner destroy \
      --provider-exe nixops4-resources-fixture \
      --type stateful_state_file \
      --inputs-json '{"name": "state.json"}' \
      --outputs-json '{}'

    [[ ! -e state.json ]]

    touch $out
  ''
