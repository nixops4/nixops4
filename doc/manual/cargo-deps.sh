#!/usr/bin/env bash
set -euo pipefail

cargoTomlFiles=()
for cargoToml in $(find ../../ -name Cargo.toml | sort); do
  if [[ "$cargoToml" == ../../Cargo.toml ]]; then
    # crate list aka "virtual manifest"; not a package
    continue
  fi
  cargoTomlFiles+=("$cargoToml")
done

manifests="$(
  for cargoToml in "${cargoTomlFiles[@]}"; do
    cargo read-manifest --offline --manifest-path "$cargoToml"
  done
)"

# Crates with `publish = false` are internal, e.g. test fixtures.
# Including them in the diagram would be a distraction.
# `publish` is `[]` for those, and `null` or a list of registries otherwise.
unpublished="$(jq -s -c '[.[] | select(.publish == []) | .name]' <<<"$manifests")"

echo '```mermaid'
echo 'graph TD;'

jq -r --argjson unpublished "$unpublished" '
    .name as $name
    | select (.name | startswith("nix-") | not)
    | select ($unpublished | index($name) | not)
    | .dependencies
    | .[]
    | select(.path)
    | select(.name | startswith("nix-") | not)
    | select(.name as $dep | $unpublished | index($dep) | not)
    | "  \($name) --> \(.name)"
  ' <<<"$manifests"


jq -r --argjson unpublished "$unpublished" '
    .name as $name
    | select (.name | startswith("nix-") | not)
    | select ($unpublished | index($name) | not)
    | "  click \(.name) \"#\(.name)\" \"\(.name)\""
  ' <<<"$manifests"

echo '```'
echo


jq -r --argjson unpublished "$unpublished" '
  select(.name | startswith("nix-") | not)
  | select(.name as $name | $unpublished | index($name) | not)
  | "### `\(.name)`\n\n\(.description)\n\n"
' <<<"$manifests"
