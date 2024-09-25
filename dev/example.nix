{ lib, inputs, ... }: {
  imports = [
    inputs.nixops4.modules.flake.default
  ];
  nixops4Deployments.default = { config, providers, withResourceProviderSystem, ... }: {
    providers.local = inputs.nixops4.modules.nixops4Provider.local;
    resources.hello = {
      type = providers.local.exec;
      inputs = {
        command = withResourceProviderSystem ({ pkgs, ... }: lib.getExe pkgs.hello);
        args = [ "--greeting" "Hallo wereld" ];
      };
    };
    resources.recycled = {
      type = providers.local.exec;
      inputs = {
        command = withResourceProviderSystem ({ pkgs, ... }: lib.getExe pkgs.hello);
        args = [ "--greeting" config.resources.hello.outputs.stdout ];
      };
    };
  };
}
