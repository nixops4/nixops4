
# Rust bindgen uses Clang to generate bindings, but that means that it can't
# find the "system" or compiler headers when the stdenv compiler is GCC.
# This script tells it where to find them.

echo "Extending BINDGEN_EXTRA_CLANG_ARGS with system include paths..." 2>&1
BINDGEN_EXTRA_CLANG_ARGS="${BINDGEN_EXTRA_CLANG_ARGS:-}"
export BINDGEN_EXTRA_CLANG_ARGS
include_paths=$(
  echo | $NIX_CC_UNWRAPPED -v -E -x c - 2>&1 \
  | awk '/#include <...> search starts here:/{flag=1;next} \
        /End of search list./{flag=0} \
        flag==1 {print $1}'
)
for path in $include_paths; do
  echo " - $path" 2>&1
  BINDGEN_EXTRA_CLANG_ARGS="$BINDGEN_EXTRA_CLANG_ARGS -I$path"
done
