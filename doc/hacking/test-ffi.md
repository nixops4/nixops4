
# Testing FFI code

If `cargo-valgrind` is broken, you may run `valgrind` manually.

1. `cd rust; cargo test -v`
2. find the relevant test suite executable in the log
    - example: `/home/user/src/nixops4/rust/target/debug/deps/nix_util-036ec381a9e3fd6d`
3. `valgrind --leak-check=full <paste the test exe>`
4. check that
    - `definitely lost: 0 bytes in 0 blocks`

## Paranoid check

Although normal valgrind tends to catch things, you may choose to enable `--show-leak-kinds=all`.
This will print a few false positive.

Acceptable leaks are those involving (and this may be Linux-specific)
- `call_init`: static initializers
  - `nix::GlobalConfig::Register::Register`
  - `_GLOBAL__sub_I_logging.cc`
  - ...
- `new<test::test_main::{closure_env#0}>`: a leak in the rust test framework

When in doubt, compare the log to a run with your new test case commented out.
