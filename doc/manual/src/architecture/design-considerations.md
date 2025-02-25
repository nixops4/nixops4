
# Does NixOps4 prevent concurrent operations on the same deployment?

Currently, this is not prevented.
The local state file is locked during individual write operations to prevent corruption, but it is not locked for the duration of a deployment.

Note that supposedly "read only" operations like checking the real-world state of a resource still produce information that should be recorded, i.e. written to the state file.

A separate lock file for `apply` operations is planned. <!-- TODO issue -->

# Can I use NixOps4 without the module system?

Yes, you can! The interface between NixOps4 and the expression language is not coupled to the module system.
You could come up with your own `mkDeployment` function that does not use the Module System at all.

Advantage:
 - You can do things differently and come up with an innovative way to connect components that doesn't use the module system.

Disadvantages:
 - Moving code between projects that use the module system and those that don't will be harder.
 - More ways to do things means more methods to learn.
 - Have to reinvent tooling around the module system, such as documentation generation.

# Can I use NixOps4 with a different configuration language?

Not yet. The Nix expression is confined to a single executable, `nixops4-eval`.
This is feasible to switch out for a different Nix implementation, or even a different language entirely.
An alternate implementation of `nixops4-eval` would only have to support the same message passing interface, but it has a lot of freedom in how it evaluates a configuration.
We will need a couple of new command line options in the `nixops4` program to pass arguments to the evaluator if the evaluator needs them, but that's easy enough.

The control flow of the evaluator is somewhat complex, as it needs to handle paused computations while figuring out what to do while waiting for input from a provider.
This can be achieved in multiple ways
- _Multi-threaded evaluation_, if it's ok to have a blocked thread per input
- Concurrent evaluation with _coroutines_
  - Make sure blackholes that can be related back to the input for which it was needed (e.g. by having a counter in the blackhole value).
    Otherwise, a dependency on an output can not be distinguished from an infinite recursion.
    This is a concern that comes up in the implementation of multi-threaded evaluation as well.
    A thread id would naturally corresponds to an input
- (Recoverable) _exceptions_
  - this is/was the initial implementation
  - ideally, partial results can be reused
    - e.g. storing the function body as a thunk instead of reverting to a thunk that represents the call
