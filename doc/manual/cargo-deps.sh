#!/usr/bin/env bash
set -euo pipefail

cargoTomlFiles=()
for cargoToml in $(find ../../ -name Cargo.toml | sort); do
  if [[ "$cargoToml" == */rust/Cargo.toml ]]; then
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

echo '```mermaid'
echo 'graph TD;'

jq -r '
    .name as $name
    | select (.name | startswith("nix-") | not)
    | .dependencies
    | .[]
    | select(.path)
    | select(.name | startswith("nix-") | not)
    | "  \($name) --> \(.name)"
  ' <<<"$manifests"


jq -r '
    .name as $name
    | select (.name | startswith("nix-") | not)
    | "  click \(.name) \"#\(.name)\" \"\(.name)\""
  ' <<<"$manifests"

echo '```'
echo


jq -r '
  select(.name | startswith("nix-") | not)
  | "# `\(.name)`\n\n\(.description)\n\n"
' <<<"$manifests"

