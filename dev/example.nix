{ lib, inputs, ... }:

let

  keys = [
    # put your public key here
    "ecdsa-sha2-nistp256 AA... user@host"
  ];

  myConfig = {
    nixpkgs.hostPlatform = "x86_64-linux";

    services.openssh.enable = true;
    services.openssh.settings.PermitRootLogin = "yes";
    networking.firewall.allowedTCPPorts = [ 22 ];

    users.users.root.openssh.authorizedKeys.keys = keys;
    users.users.root.initialPassword = "asdf";
    users.users.user.openssh.authorizedKeys.keys = keys;
    users.users.user.initialPassword = "asdf";
    users.users.user.isNormalUser = true;
    users.users.user.group = "user";
    users.groups.user = { };
  };

  defaultHostPort = 2222;

in
{
  imports = [
    inputs.nixops4.modules.flake.default
  ];
  nixops4Deployments.default = { config, providers, withResourceProviderSystem, ... }:
    let inherit (lib) mkOption types;
    in {
      options = {
        hostPort = mkOption {
          type = types.port;
          description = ''
            The port on the host to forward to the guest's SSH port.
          '';
          default = defaultHostPort;
        };
      };
      config = {
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
        resources.nixos = {
          type = providers.local.exec;
          imports = [
            inputs.nixops4.modules.nixops4Resource.nixos
          ];

          nixpkgs = inputs.nixpkgs;
          nixos.module = { pkgs, modulesPath, ... }: {
            # begin hardware config
            imports = [
              (modulesPath + "/profiles/qemu-guest.nix")
              myConfig
            ];
            fileSystems."/".device = "/unknown";
            boot.loader.grub.enable = false;
            # end hardware config

            environment.etc."greeting".text = config.resources.hello.outputs.stdout;
            environment.systemPackages = [
              pkgs.hello
            ];
          };

          ssh.opts = "-o Port=${toString config.hostPort}";
          ssh.host = "127.0.0.1";
          ssh.hostPublicKey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAiszi43aOWWV7voNgQ1Ifa7LGKwGJfOuiLM1n42h2Y8";
        };
      };
    };

  /**
    A VM configuration to try the deployment

        $ nix run .#example-vm

    Log in as root, with password asdf

    Retrieve the host key from within the VM

        $ ssh-keyscan localhost

    Paste the part starting with `ssh-ed25519` into `ssh.hostPublicKey` in the resource.

    Deploy!

        $ nixops4 apply

    Clean up

        $ rm nixos.qcow2
  */
  flake.apps.x86_64-linux.example-vm =
    let
      baseConfiguration = inputs.nixpkgs.lib.nixosSystem {
        modules = [
          # (inputs.nixpkgs + "/nixos/modules/virtualisation/qemu-guest.nix")
          {

            imports = [ myConfig ];

            virtualisation.vmVariant = {
              virtualisation = {
                # Non-graphical: easier copy paste
                qemu.consoles = [ "ttyS0,115200n8" ];
                graphics = false;

                memorySize = 4096;
                diskSize = 10 * 1024;
                forwardPorts = [
                  { host.port = defaultHostPort; guest.port = 22; }
                ];
              };
            };
          }
        ];
      };
      config = baseConfiguration.config.virtualisation.vmVariant;
    in
    {
      type = "app";
      program = config.system.build.vm;
    };
}
