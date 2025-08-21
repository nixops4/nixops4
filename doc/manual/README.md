# `doc/manual`

This directory contains the manual, which is deployed to [`nixops.dev`](https://nixops.dev).

## Building

Quick one-off: `nix run .#open-manual`

Iterating and/or inspecting intermediate results:

```bash
nix develop
cd doc/manual
make open
# iterate again:
make
# refresh the page
```

Link check (internal only):

```bash
nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).manual-links
```
