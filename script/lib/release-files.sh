# The stand-in releases of the tests of the installers and the npm package, and the server of their
# files, sourced by script/installers-test and script/npm-package-test. Each defines fail <message>.

source "$(dirname "${BASH_SOURCE[0]}")/start-server.sh"

windows=false
case $(uname -s) in
  MINGW* | MSYS* | CYGWIN*) windows=true ;;
esac
# native <path>: the path as node and npm on this OS take it.
native() {
  if $windows; then cygpath -m "$1"; else printf '%s' "$1"; fi
}
# sha256 <file>: the file's SHA-256 in hex.
sha256() {
  node -e 'process.stdout.write(require("crypto").createHash("sha256").update(require("fs").readFileSync(process.argv[1])).digest("hex"))' "$(native "$1")"
}
# checksum <archive>: writes <archive>.sha256 as dist writes it: `<hex> *<name>` and a blank line.
checksum() {
  printf '%s *%s\n\n' "$(sha256 "$1")" "${1##*/}" >"$1.sha256"
}

# serve <dir> <log> <var> [<cert> <key>]: serves the files of <dir> as GitHub serves release files,
# script/lib/serve.mjs, over HTTPS with the certificate <cert> and its key <key>, each request's
# path a line of <log> after the port; adds the server's PID to servers and sets the variable <var>
# to its address, http://127.0.0.1:<port>. Exits 1 when it does not start.
servers=()
serve() {
  local started=true tls=()
  [[ $# -eq 3 ]] || tls=("$(native "$4")" "$(native "$5")")
  start_server "$2" node "$(native "$(dirname "${BASH_SOURCE[0]}")/serve.mjs")" "$(native "$1")" \
    ${tls[@]+"${tls[@]}"} || started=false
  servers+=("$server_pid")
  $started || {
    echo "${0##*/}: the server of the release files did not start: $server_failure" >&2
    exit 1
  }
  printf -v "$3" '%s' "http://127.0.0.1:$server_port"
}

# stand_in <dir> <target> <line>: a skald laid out as dist lays it out in <dir>/skald-client-<target>,
# which prints <line> for --version, `args:` and each argument in brackets for anything else, and
# exits with <n> for `exit-with <n>`. Each run appends its own path, symlinks resolved, to the file
# $STAND_IN_RAN.
stand_in() {
  mkdir -p "$1/skald-client-$2"
  cat >"$1/skald-client-$2/skald" <<EOF
#!/bin/sh
if [ -n "\${STAND_IN_RAN:-}" ]; then
  printf '%s\n' "\$(cd "\$(dirname "\$0")" && pwd -P)/\$(basename "\$0")" >>"\$STAND_IN_RAN"
fi
case \${1:-} in
  --version) echo "$3" ;;
  exit-with) exit "\$2" ;;
  *)
    printf 'args:'
    for arg in "\$@"; do printf ' [%s]' "\$arg"; done
    echo
    ;;
esac
EOF
  chmod 755 "$1/skald-client-$2/skald"
}

# stand_in_release <dir> <version>: the stand-in release <dir>/release: an archive of each macOS and
# Linux target, laid out as dist lays it out, with its checksum, whose skald, a stand-in from
# <dir>/build, prints `skald <version> <target>`.
stand_in_release() {
  local t
  mkdir -p "$1/release"
  for t in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
    stand_in "$1/build" "$t" "skald $2 $t"
    tar -czf "$1/release/skald-client-$t.tar.gz" -C "$1/build" "skald-client-$t"
    checksum "$1/release/skald-client-$t.tar.gz"
  done
}

# The refused releases: one whose checksum does not match the archive, one with no checksum, one
# with an empty checksum, and one whose archive was replaced under its checksum.
refused=(mismatched missing empty replaced)
# refusal <case>: what the refused release <case> has.
refusal() {
  case $1 in
    mismatched) echo 'a checksum that does not match the archive' ;;
    missing) echo 'no checksum of the archive' ;;
    empty) echo 'an empty checksum of the archive' ;;
    replaced) echo 'an archive replaced under its checksum' ;;
  esac
}
# refused_releases <release> <archive> <target> <dir> [<file>...]: each refused release in
# <dir>/<case>: a copy of the <archive> of <release> for <target>, its checksum and its <file>s,
# each one it has, the checksum then broken as <case> says.
refused_releases() {
  local release=$1 archive=$2 target=$3 dir=$4 case file
  shift 4
  for case in "${refused[@]}"; do
    mkdir "$dir/$case"
    for file in "$archive" "$archive.sha256" "$@"; do
      [[ ! -f $release/$file ]] || cp "$release/$file" "$dir/$case/"
    done
  done
  # The SHA-256 of nothing: a well-formed checksum, of another file.
  printf '%s *%s\n\n' e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855 "$archive" \
    >"$dir/mismatched/$archive.sha256"
  rm -f "$dir/missing/$archive.sha256"
  : >"$dir/empty/$archive.sha256"
  # A skald of its own, laid out as the release's, under the release's checksum.
  rm -f "$dir/replaced/$archive"
  if $windows; then
    mkdir "$dir/replacement"
    echo 'skald replaced' >"$dir/replacement/skald.exe"
    powershell -NoProfile -Command "Compress-Archive -Force -Path '$(cygpath -w "$dir/replacement/skald.exe")' -DestinationPath '$(cygpath -w "$dir/replaced/$archive")'"
  else
    stand_in "$dir/replacement" "$target" 'skald replaced'
    tar -czf "$dir/replaced/$archive" -C "$dir/replacement" "skald-client-$target"
  fi
  [[ $(sha256 "$dir/replaced/$archive") != "$(sha256 "$release/$archive")" ]] ||
    fail "the replaced archive is the release's own"
}
