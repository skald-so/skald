# The names and helpers the Release PR workflow and its tests share, sourced by
# script/release-update, script/install-git-cliff and their tests. The GitHub App skald-release, its environment and its
# secrets are set up on GitHub under these names; a rename changes them here and in the workflow.

# The workflow that tags the merge of a Release PR and keeps the Release PR open.
release_workflow=.github/workflows/release-pr.yml
# The GitHub environment with the App's secrets; only dev deploys from it, only the jobs of that
# workflow use it.
release_environment=release-plz
# The App's ID and private key, secrets of that environment.
release_app_id_secret=RELEASE_APP_ID
release_app_key_secret=RELEASE_APP_PRIVATE_KEY
# The git-cliff the workflow and the tests run: the official release archive for the machine,
# installed by script/install-git-cliff after a check against its SHA-256 here, into a directory of
# its own, first on PATH here.
git_cliff_version=2.14.2
git_cliff_archives=(
  "x86_64-unknown-linux-musl fa21e2fc70046fcfd0e43e5e4613873ab799aff3d9dc8559c00d31b270e417d0"
  "aarch64-unknown-linux-musl 4be21b21ec9699af4d330b224aeba8a41d41d8f5b351ca2da8a9ea2bdc36380f"
  "x86_64-apple-darwin 4e647d2deb152aca1e987f225cf42b080b991dd98f3bce4027ef95446d982ac3"
  "aarch64-apple-darwin a0be97e237d440d34c5e508ec21d070b2c85cbb8be48a46a56f9a4e8bd247f05"
)
git_cliff_url=https://github.com/orhun/git-cliff/releases/download
git_cliff_root=${CARGO_HOME:-$HOME/.cargo}/skald/git-cliff-$git_cliff_version
PATH=$git_cliff_root/bin:$PATH

# set_version <dir> <version>: sets the workspace version in [workspace.package] of <dir>/Cargo.toml
# and of every crate of the workspace, the packages without a source, in <dir>/Cargo.lock. It needs
# no cargo registry. False, changing nothing, when Cargo.toml has no such version.
set_version() {
  awk -v version="$2" '
    /^\[/ { section = $0 }
    section == "[workspace.package]" && /^version *=/ { $0 = "version = \"" version "\""; set = 1 }
    { print }
    END { exit !set }
  ' "$1/Cargo.toml" >"$1/Cargo.toml.new" || {
    rm -f "$1/Cargo.toml.new"
    return 1
  }
  awk -v version="$2" '
    held != "" { print ($0 ~ /^source = / ? held : "version = \"" version "\""); held = "" }
    name && /^version = / { held = $0; name = 0; next }
    { name = /^name = /; print }
    END { if (held != "") print "version = \"" version "\"" }
  ' "$1/Cargo.lock" >"$1/Cargo.lock.new"
  mv "$1/Cargo.toml.new" "$1/Cargo.toml"
  mv "$1/Cargo.lock.new" "$1/Cargo.lock"
}

# fixture_git <dir>: isolates the git of the release fixtures in <dir> from the user's config and
# from any repository around <dir>.
fixture_git() {
  export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
  GIT_CEILING_DIRECTORIES=$(cd "$1" && pwd -P)
  export GIT_CEILING_DIRECTORIES
  export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com
  export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com
}

# copy_tree <repository> <dir>: copies the working tree of <repository> as script/test sees it into a
# new <dir>, for the release fixtures: tracked and new files, without ignored ones and without .git.
copy_tree() {
  mkdir "$2"
  (
    cd "$1"
    git ls-files -z --cached --others --exclude-standard | while IFS= read -r -d '' path; do
      if [[ -e $path || -L $path ]]; then printf '%s\0' "$path"; fi
    done | tar --null -T - -cf -
  ) | tar -C "$2" -xf -
}
