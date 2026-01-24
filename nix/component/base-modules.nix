{
  _class = "nixops4Component";

  imports = [
    ./resource.nix
    ./provider-declarations.nix
    ./provider-values.nix
    ./export.nix
    ./members.nix
  ];
}
