{
  _class = "nixops4Deployment";

  imports = [
    ./export.nix
    ./resources.nix
    ./providers.nix
    ./deployments.nix
  ];
}
