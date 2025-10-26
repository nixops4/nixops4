{
  outputs =
    { ... }:
    {
      modules.flake-parts.default = { };
      modules.flake.default = { };
      flakeModules.default = { };
      flakeModule = { };
    };
}
