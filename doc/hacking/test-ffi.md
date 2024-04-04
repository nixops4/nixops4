
# Testing FFI code

If `cargo-valgrind` is broken, you may run `valgrind` manually.

1. `cd rust; cargo test -v`
2. find the relevant test suite executable in the log
    - example: `/home/user/src/nixops4/rust/target/debug/deps/nix_util-036ec381a9e3fd6d`
3. `valgrind --suppressions=../test/valgrind/valgrind.suppressions <paste the test exe>`
4. check that
    - `definitely lost: 0 bytes in 0 blocks`

# Leaks

Pass `--leak-check=full` to `valgrind` to check for memory leaks.
This currently produces a lot of positives due to Nix's lack of cleanup around `EvalState`.
