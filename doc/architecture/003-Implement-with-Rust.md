# Implement with Rust

## Context

Due to significant architectural changes, NixOps 4 is a complete rewrite.
This gives the project an opportunity to choose a different programming language.

## Decision

NixOps 4 will be implemented in Rust.

- (+) General purpose language suitable for the purpose ("systems programming")
- (+) Memory safety
- (+) No garbage collector, makes integrating with a garbage-collected language easier
- (+) Good performance
- (+) Good Nix packaging, with multiple integrations available, which align with local development method
- (+) Nix bindings available, although they will need to be improved, but they are mostly de-risked
- (+) Rust has precedent in the Nix community
  - nixpkgs-fmt, alejandra
  - rnix-lsp, nil
  - tvix
  - nixpkgs-check-by-name
  - Nix itself has experimented with a gradual Rust migration, although this was reverted because the gradual migration process was not viable
- (-) Somewhat steep learning curve
  - Less than Haskell's
  - Partially mitigated by AI assistants

## Alternatives

### Python

- (~) NixOps 1 and 2 were implemented in Python, but continuity is not relevant because NixOps 4 is a complete rewrite
- (+) Python also has bindings available
- (+) Python is considered to be easy to write
  - This is less of a priority, as most development will happen in resource implementations and Nix code, which are independent of the NixOps implementation language choice.
- (-) Python types are not as strong as Rust's.
  - NixOps 2 showed that gradual typing + ratchet make contributing somewhat harder.
    CI failures should be binary - not "maybe you should do a better job at typing, so let's call that a failure".
    The alternative, no ratchet, is not acceptable, because type quality would deteriorate.
  - Types seem to change ("improve") for no clear reason
- (-) Python has worse performance - a risk, not necessarily a problem
- (-) Packaging Python with Nix has proved challenging. Use of poetry2nix in Nixpkgs was discontinued, leading to a situation where the Nixpkgs packaging and local packaging would continually diverge. Python-native dependency management was not an option anymore.

### Haskell

- (+) Haskell has strong types
- (+) Haskell has good performance
- (+) Haskell has good Nix bindings (`hercules-ci-cnix-expr`), with possibly no changes needed to them
  - (-) if well written
- (-) Haskell has a steep learning curve
- (-) Haskell has a its own garbage collector, making integration of the Nix language potentially riskier
  - (+) This has proven to be a non-issue in `hercules-ci-agent`
- (-) Haskell has a smaller community than Rust, which could make it harder to find contributors
- (~) Haskell can be slow to iterate on, unless most iteration can happen through unit tests, which may be tricky for NixOps, of which a large part is integration testing.

### C++

- (+) C++ has good performance.
- (+) C++ is somewhat easy to package. Local packaging would be the same as Nixpkgs.
- (-) C++ ecosystem in general still doesn't have a good packaging story yet.
- (-) C++ does not have Nix bindings.
  - Nix is implemented in C++, but that C++ is not a stable interface. It still needs C++ bindings based on the C interface, which do not exist, and would seem unappealing.
- (-) C++ has worse memory safety.
- (-) C++ has worse types.
- (-) C++ is not as appealing to contributors.
- (-) C++ is harder to review.

### Go

- (+) Go has good performance.
- (+) Go has Nix bindings.
  - (?) Quality and completeness are unknown - a risk.
- (+) Good Nix packaging, matching local development
- (+) Memory safety
- (-) Go has worse types
- (-) Go has its own garbage collector, making integration of the Nix language potentially riskier (and not de-risked like Haskell's)
- (~) Go seems to be somewhat of a polarizing language, being called simplistic, making it unappealing to some contributors, particularly to those with experience in Rust, Haskell, or C++, which are well represented in the Nix community

# JavaScript

- (+) Popular language
- (+) Memory safety
- (-) Nix packaging is not great, as it would incentivize a split between local packaging and Nixpkgs packaging.
- (?) No bindings known
- (-) Node.js has its own garbage collector, making integration of the Nix language potentially riskier
- (~) TypeScript may provide types
  - (+) Mature
  - (-) Complicated
  - (-) Unsound
  - (~) Perhaps controversial among potential contributors
- (?) JS community is large, but perhaps underrepresented in the Nix community

# Java

- (-) Java startup is too slow for CLI tools
- (-) Java packaging in Nix is sub-par

# C#

- (-) Impopular in open source community

# Other interpreted language

Arguments somewhat similar to Python, but less popular language.

# Other compiled language

Not as popular or safe as those already mentioned.
