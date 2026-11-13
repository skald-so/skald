# check_release_tag_args <args>...: exits with usage status 2 unless the arguments are <tag> <dir>,
# <tag> a release tag, vX.Y.Z or vX.Y.Z-beta.N. Sourced by script/release-installers and
# script/release-npm.
check_release_tag_args() {
  if [[ $# -ne 2 ]]; then
    echo "usage: $0 <tag> <dir>" >&2
    exit 2
  fi
  if [[ ! $1 =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-beta\.[0-9]+)?$ ]]; then
    echo "${0##*/}: $1 is no release tag, vX.Y.Z or vX.Y.Z-beta.N" >&2
    exit 2
  fi
}
