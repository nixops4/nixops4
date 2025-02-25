
# Hacking on `nixops4`

NixOps4 can be worked on in three main ways:

1. Unit test driven with Cargo, recommended for small changes and new Rust components
2. Integration test driven with Nix
3. Manual testing with Cargo, more suitable for iterating on UX, but not so much for making sustainable changes

## Unit testing

```console
nix develop
cargo test
```

## Integration testing

```console
nix build .#checks.x86_64-linux.something
```

Use tab completion to see the available checks instead of `something`.

<!-- TODO: tricks for doing eval in the offline VM -->

## Manual testing

```console
nix develop
cargo install --path rust/nixops4 && cargo install --path rust/nixops4-eval
```

In the same shell or a different terminal, you can `cd` into a project and run `nixops4` as you would normally.

```console
PATH="$HOME/.cargo/bin:$PATH"
```
