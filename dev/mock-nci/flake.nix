{
  inputs = { };
  outputs =
    { ... }:
    {
      flakeModule =
        { ... }:
        {
          perSystem =
            { lib, ... }:
            {
              options.nciIsMocked = lib.mkOption { };
              options.nci = lib.mkOption { };
            };
        };
    };
}
