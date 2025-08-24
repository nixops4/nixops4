#!/usr/bin/env bash

# This script renders the test case and overwrites the expected files to match.
# Use this when a small change is detected which is acceptable.
# Large changes should be reviewed thoroughly for completeness and for correct
# rendering on the docs site.

set -euo pipefail

# Get the directory where this script is located
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
expected_dir="${script_dir}/expected"
rendered_docs="${script_dir}/actual.built.tmp"

echo "Building test case to get rendered docs..."

# Navigate to the repository root
cd "${script_dir}/../.."

# Ensure cleanup happens even if script fails
trap 'rm -f "$rendered_docs"' EXIT

# Build the test and get the rendered docs via passthru
nix build --extra-experimental-features 'nix-command flakes' \
  --out-link "$rendered_docs" \
  ".#checks.$(nix eval --impure --raw --expr builtins.currentSystem).render-provider-docs.actual"

if [ ! -d "$rendered_docs" ]; then
  echo "ERROR: Failed to build rendered docs"
  exit 1
fi

echo "Rendered docs are at: $rendered_docs"

# Remove and recreate expected directory
echo "Cleaning old expected directory..."
rm -rf "$expected_dir"

# Copy entire directory, dereferencing the Nix output symlink
echo "Copying rendered files to expected directory..."
cp -r --dereference "$rendered_docs" "$expected_dir"

# Make the copied files writable so they can be easily edited/deleted later
chmod -R +w "$expected_dir"

echo ""
echo "Successfully adopted all changes!"
echo "Expected files are now in: $expected_dir"
echo ""
echo "Files adopted:"
ls -la "$expected_dir"
echo
echo "Make sure to stage all changes with git."
