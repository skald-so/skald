# test_stand_ins <dir>: a copy of script/test in <dir> that runs no check, for the tests of which
# checks it runs: <dir>/script/test with script/changed-parts and stand-ins for everything else it
# runs, the other scripts, PostgreSQL's functions and, in <dir>/bin, the tools. Each stand-in that
# runs a check appends it to <dir>/checks as `<directory>\t<command>`, the directory relative to
# <dir>; those of the tools only the prerequisites probe, rustc, wasm-bindgen-test-runner and
# python3, append nothing. git is the real one. Run it as PATH="<dir>/bin:$PATH" <dir>/script/test.
# Sourced by script/changed-parts-test; script/ci-workflow-test runs it with bash.
test_stand_ins() {
  local root dir name
  root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
  mkdir -p "$1"
  dir=$(cd "$1" && pwd -P)
  mkdir -p "$dir/script/lib" "$dir/bin" "$dir/web" "$dir/prototypes/mls" \
    "$dir/sysroot/lib/rustlib/wasm32-unknown-unknown"
  cp "$root/script/test" "$root/script/changed-parts" "$dir/script/"
  echo 'postgres_start() { :; }; postgres_wait() { :; }; postgres_stop() { :; }' \
    >"$dir/script/lib/postgres.sh"
  printf 'name = "wasm-bindgen"\nversion = "0.0.0"\n' >"$dir/prototypes/mls/Cargo.lock"
  for name in "$root"/script/*; do
    name=${name#"$root"/}
    if [[ -f $root/$name && $name != script/test && $name != script/changed-parts ]]; then
      stand_in "$dir" "$name" "$name"
    fi
  done
  for name in cargo cargo-deny cargo-mutants node npm npx jq curl docker openssl tar \
    ssh ssh-add ssh-agent ssh-keygen sshd; do
    stand_in "$dir" "bin/$name" "$name"
  done
  printf '#!/usr/bin/env bash\necho %q\n' "$dir/sysroot" >"$dir/bin/rustc"
  printf '#!/usr/bin/env bash\necho wasm-bindgen-test-runner 0.0.0\n' \
    >"$dir/bin/wasm-bindgen-test-runner"
  printf '#!/usr/bin/env bash\n' >"$dir/bin/python3"
  chmod +x "$dir"/bin/* "$dir"/script/*
}

# stand_in <dir> <path> <command>: a stand-in at <dir>/<path> that appends `<command> <arguments>`
# to <dir>/checks, after the directory it runs in.
stand_in() {
  printf '#!/usr/bin/env bash\nhere=$(pwd -P)/\nhere=${here#%q/}\nprintf "%%s\\t%%s\\n" "${here%%/}" "%s${*:+ $*}" >>%q\n' \
    "$1" "$3" "$1/checks" >"$1/$2"
}
