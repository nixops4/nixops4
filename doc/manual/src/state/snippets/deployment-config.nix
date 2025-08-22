{
  # Define the state provider resource
  resources.myStateFile = {
    type = providers.local.state_file;
    inputs.name = "deployment-state.json";
  };

  # Use it for a stateful resource.
  # A generated database URL and such will be stored in myStateFile.
  resources.myDatabase = {
    type = providers.cloud.database;
    state = "myStateFile"; # References the state provider resource above
    inputs = {
      size = "db.t3.micro";
      engine = "postgres";
      # ... other database configuration
    };
  };
}
