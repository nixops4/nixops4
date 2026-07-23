# Root Expression

The _root expression_ is a deployment's entry point: a single Nix expression describing all of your [resources][resource] and how they connect. `nixops4` evaluates it to learn what your deployment consists of.

It must evaluate to a _root component_, which you normally create with [`nixops4.lib.mkRoot`][mkRoot]:

```nix
nixops4.lib.mkRoot {
  modules = [
    ({ providers, members, ... }: {
      # your providers and resources go here
    })
  ];
}
```

You rarely load it by hand; the [`nixops4`][nixops4-cli] command discovers it for you, from either a flake or a plain Nix file.

## Where NixOps4 looks

`nixops4` selects the root source in this order:

1. **[`--file <PATH>`](#from-a-file)**, if you pass it.
2. **`nixops4.nix`** in the current directory, if that file exists.
3. Otherwise, the **flake** in the current directory.

The first match wins: a `nixops4.nix` is used even when the directory also contains a `flake.nix`.

## From a flake

In this case, NixOps4 reads the current directory's flake and takes its **`nixops4`** output attribute, which must be a root component.

An easy way to produce that output is the flake-parts module, which lets you write your deployment as [a module][module-options] and wires up the `nixops4` output for you:

```nix
{
  inputs.nixops4.url = "...";
  outputs = inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [ inputs.nixops4.modules.flake.default ];

      nixops4 = { providers, members, ... }: {
        imports = [ ./deployment.nix ];
        providers.local = inputs.nixops4.modules.nixops4Provider.local;
        members.myDeployment = {
          # ...
        };
      };
    };
}
```

**Check that:**
- you don't call `mkRoot` — the module does it for you
- `nixops4` is at the top level, not `flake.nixops4`

Without flake-parts, produce the `nixops4` output directly:

```nix
{
  inputs.nixops4.url = "...";
  outputs = inputs@{ ... }: {
    nixops4 = inputs.nixops4.lib.mkRoot {
      modules = [
        ./deployment.nix
        ({ providers, members, ... }: {
          providers.local = inputs.nixops4.modules.nixops4Provider.local;
          members.myDeployment = {
            # ...
          };
        })
      ];
    };
  };
}
```

**Check that:**
- `nixops4` is at the top level
- you *do* call `mkRoot`
- `mkRoot`'s argument is `{ modules = [ … ]; }` — your module goes in the `modules` list, not as the argument itself

### Overriding flake inputs

When loading from a flake, you can swap an input without editing `flake.nix`:

```console
$ nixops4 apply --override-input nixpkgs ~/src/nixpkgs
```

It takes an input attribute path and a flake reference, and applies only to flake loading — so it cannot be combined with [`--file`](#from-a-file) or a discovered `nixops4.nix`.

## From a file

Point NixOps4 at a plain Nix file with `--file`, or place a `nixops4.nix` in the current directory:

```console
$ nixops4 apply --file ./my-deployment.nix
```

The file must evaluate to a root component, just like the flake's `nixops4` output.

A plain file has no inputs, so NixOps4 does **not** inject `nixops4.lib`; the file must obtain `nixops4` itself, for instance by importing it:

```nix
let
  nixops4 = import ./nixops4-deps.nix;
in
nixops4.lib.mkRoot {
  modules = [
    ({ providers, members, ... }: {
      providers.local = nixops4.modules.nixops4Provider.local;
      members.myDeployment = {
        # ...
      };
    })
  ];
}
```

For the same reason, flake-specific flags like `--override-input` don't apply in file mode; passing them with a discovered `nixops4.nix` is an error.

[resource]: ./concept/resource.md
[mkRoot]: ./lib/index.md#mkRoot
[nixops4-cli]: ./cli/nixops4.md
[module-options]: ./modules/index.md
