{
  # TODO: rename to nixops4Component when root deployment is also a component (ADR 009)
  _class = "nixops4Deployment";

  imports = [
    # _export is provided by componentExport in members.nix
    ./providers.nix
    ./members.nix
  ];
}
