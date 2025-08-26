{
  # Define the state provider resource
  resources.myStateFile = {
    type = providers.local.state_file;
    inputs.name = "deployment-state.json";
  };

  # Use it for a stateful resource.
  # A generated unique ID (for example) will be stored in myStateFile.
  resources.myReverseProxy = {
    type = providers.cloud.reverse_proxy;
    state = "myStateFile"; # References the state provider resource above
    inputs = {
      rules = [
        # ...
      ];
    };
  };
}
