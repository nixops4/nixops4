{ runCommand }:

runCommand "check-representations" { } ''
  (
    set -uxo pipefail

    fail() {
      printf 'error: %s\n' "$*" >&2
      exit 1
    }

    engineCargoToml=${../rust/nixops4/Cargo.toml}
    engineSrc=${../rust/nixops4/src}
    runnerSrc=${../rust/nixops4-resource-runner/src}
    wireCargoToml=${../rust/nixops4-provider-wire/Cargo.toml}
    memberCargoTomls=$(echo ${../rust}/*/Cargo.toml)

    engineDeps=$(sed '/^[[:space:]]*#/d' "$engineCargoToml")
    if grep -F 'nixops4-provider-wire' <<<"$engineDeps"; then
      fail "nixops4 must not depend on nixops4-provider-wire (the wire types); use the runner's client API instead"
    fi
    if grep -F 'path = "../nixops4-resource"' <<<"$engineDeps"; then
      fail "nixops4 must not depend on nixops4-resource (the SDK); use the runner's client API instead"
    fi

    if grep -rnP 'nixops4_provider_wire|nixops4_resource(?!_runner)' "$engineSrc"; then
      fail "nixops4 sources must not name nixops4_provider_wire or nixops4_resource"
    fi

    runnerLibs=$(find "$runnerSrc" -name '*.rs' ! -name 'main.rs')
    if ! awk '
      function flush() {
        if (item ~ /v0::|nixops4_provider_wire/) {
          print FILENAME ": " item
          bad = 1
        }
        item = ""
      }
      /^[[:space:]]*pub / {
        if (initem) flush()
        item = $0
        if ($0 ~ /[{;]/) { flush(); initem = 0 } else { initem = 1 }
        next
      }
      initem {
        item = item " " $0
        if ($0 ~ /[{;]/) { flush(); initem = 0 }
      }
      END {
        if (initem) flush()
        exit bad ? 1 : 0
      }
    ' $runnerLibs; then
      fail "the public API of nixops4-resource-runner's library must not mention wire types"
    fi

    for cargoToml in $memberCargoTomls; do
      case "$cargoToml" in
        */nixops4-resource/Cargo.toml|*/nixops4-resource-runner/Cargo.toml|*/nixops4-provider-wire/Cargo.toml) ;;
        *)
          if grep -F 'nixops4-provider-wire' "$cargoToml"; then
            fail "only nixops4-resource and nixops4-resource-runner may depend on nixops4-provider-wire: $cargoToml"
          fi
          ;;
      esac
    done

    if grep -F 'path = "../nixops4' "$wireCargoToml"; then
      fail "nixops4-provider-wire must not depend on other workspace crates"
    fi
  )
  touch $out
''
