# Testing a Resource Provider

NixOps4 resource providers can be tested in multiple ways.

## Choose a Testing Environment

The choice of testing method depends on the desired trade-offs familiarity and convenience vs. robustness.

The following table provides an overview of the trade-offs, which are explained in more detail below.

Criteria:

📦: Is the test hermetic and reproducible? <br/>
❄️: Is it easy to set up NixOS services? <br/>
☁️: Can the test access the network? <br/>
🏗️: Can the test build derivations? <br/>
🚚: Can the test use a Nix cache? <br/>
🍏📦: Can it test a macOS build of the application under test? <br/>
🍏🧑‍💻: Can a macOS user test with it?


<!-- | Environment | Test runner | Hermetic | Network access | Can build | Can use cache | Tests macOS build | Runs on macOS | Status / notes | -->
| Environment                         | Runner                 | 📦 | ❄️  | ☁️  | 🏗️ | 🚚 | 🍏📦 | 🍏🧑‍💻 | notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Nix sandbox                         | [`nixops4-resource-runner`](#nixops4-resource-runner) | ✅ | ❌ | ❌ | ❌ | ❌  | ✅ | ✅  | |
| Nix sandbox with different storeDir | `nixops4`                   | ✅ | ❌ | ❌ | ✅ | ❌  | ✅ | ✅  | Impractical |
| Nix sandbox with relocated store    | `nixops4`                   | ✅ | ❌ | ❌ | ✅ | ✅¹ | ❌ | ❌  | 🚧 Untested |
| Nix sandbox with recursive nix      | `nixops4`                   | ⚠️³ | ❌ | ❌ | ✅ | ✅  | ✅ | ✅  | ⚠️³ |
| NixOS VM test                       | `nixops4`                   | ✅ | ✅ | ❌ | ✅ | ✅¹ | ❌ | ✅² | 🚧 In development, adds ~10s overhead |
| Unsandboxed                         | either                      | ❌ | ❌ | ✅ | ✅ | ✅  | ✅ | ✅  | Not perfect, but can be good |

¹: Make sure to add expected build inputs to the check derivation or [`system.extraDependencies`][nixos-extraDependencies]

²: Requires a "remote" builder, which can be provided by [nix-darwin]'s [`nix.linux-builder.enable`](https://daiderd.com/nix-darwin/manual/index.html#opt-nix.linux-builder.enable)

³: The [`recursive-nix`][recursive-nix] experimental feature is not planned to be supported in the long term and [has problems](https://github.com/NixOS/nix/labels/recursive-nix).

### Environment

The main differentiator is the environment. The benefits of picking a more restrictive environment include
- ability to run offline
- hermeticity and the ability to `git bisect`

These tend to be lost when running outside the Nix sandbox.

If you are testing a provider that interacts with the network, you may have no choice.

### Test runner

You may run your tests with `nixops4` or `nixops4-resource-runner`. The latter is simpler and easy to call from a script, and is good for a "unit test" style of testing, whereas `nixops4` proper makes it easy to test whole deployments.

### Can build

If your test relies on building a derivation, this may be a deciding factor. The Nix sandbox does not normally allow building, but workarounds exist.

Many providers do not require building to test them.

### Can use cache

This is only relevant if you are building derivations in the test. Depending on the workaround, you may be able to use pre-built dependencies.

### MacOS support

We can distinguish between the ability to test a provider that is built for macOS, versus the ability to test using macOS at all.

A NixOS VM test can be run on a macOS host, but it will not test the provider on macOS.

[recursive-nix]: https://nix.dev/manual/nix/latest/development/experimental-features#xp-feature-recursive-nix
[nix-darwin]: https://daiderd.com/nix-darwin/
[nixos-extraDependencies]: https://search.nixos.org/options?show=system.extraDependencies&sort=relevance&query=extraDependencies
[`nixops4-resource-runner`]: ../cli/nixops4-resource-runner.md

## Testing with nixops4-resource-runner {#nixops4-resource-runner}

The [`nixops4-resource-runner`] tool provides a simple way to test resource providers by invoking all provider operations directly. See the [resource provider interface](./interface.md) for details about the protocol.

### Example: Testing a stateless resource

```bash
nixops4-resource-runner create \
  --provider-exe nixops4-resources-local \
  --type file \
  --input-str name test.txt \
  --input-str contents "Hello, world!"
```

See [`nixops4-resource-runner create`](../cli/nixops4-resource-runner.md#nixops4-resource-runner-create).

### Example: Testing a stateful resource

```bash
# Create with state persistence
nixops4-resource-runner create \
  --provider-exe nixops4-resources-local \
  --type memo \
  --stateful \
  --input-json initialize_with '"initial value"'

# Update the stateful resource
nixops4-resource-runner update \
  --provider-exe nixops4-resources-local \
  --type memo \
  --inputs-json '{"initialize_with": "new value"}' \
  --previous-inputs-json '{"initialize_with": "initial value"}' \
  --previous-outputs-json '{"value": "initial value"}'
```

The `--stateful` flag indicates that state persistence will be provided to the resource. Resources that require state must fail if this flag is not set.

See [`nixops4-resource-runner create`](../cli/nixops4-resource-runner.md#nixops4-resource-runner-create) and [`nixops4-resource-runner update`](../cli/nixops4-resource-runner.md#nixops4-resource-runner-update).
