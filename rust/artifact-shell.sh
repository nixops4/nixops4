#!/usr/bin/env false
# This script is intended to be sourced, not executed directly.
# shellcheck shell=bash

# This script is used to set up the environment that includes
# - the debug builds from the rust/ directory (this directory)
# - bash completion

rust_artifact_shell_setup() {
  local here="$PWD"
  if [[ -d "$here/rust/nixops4" ]]; then
    here="$here/rust"
  fi
  if ! [[ -d "$here/nixops4" ]] && ! [[ -f "$here/../flake.nix" ]]; then
    echo >&2 "This script must be run from the nixops4/rust directory"
    return 1
  fi
  echo >&2 "Extending PATH to include rust debug builds"
  export PATH="$PATH:$here/target/debug"

  echo >&2 "Extending XDG_DATA_DIRS to include our autocompletions"
  export XDG_DATA_DIRS="$here/target/_nixops4_xdg_data:$XDG_DATA_DIRS"

  if [[ -x "$here/target/debug/nixops4" ]]; then
    echo >&2 "Refreshing bash completion scripts"
    mkdir -p "$here/target/_nixops4_xdg_data/bash-completion/completions"
    "$here/target/debug/nixops4" generate-completion --shell bash \
      > "$here/target/_nixops4_xdg_data/bash-completion/completions/nixops4"
  else
    echo >&2 -e "\033[1;35mwarning:\033[0m nixops4 not built yet, skipping completion generation"
    echo >&2 "To refresh shell completions, run"
    echo >&2 "  (cd rust; cargo build)"
    echo >&2 "  source rust/artifact-shell.sh"
  fi
}
rust_artifact_shell_setup
