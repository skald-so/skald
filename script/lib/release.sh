# The names the release-plz workflow and its tests share, sourced by script/release-plz-test,
# script/release-plz-workflow-test, script/install-release-plz and the release tag tests. The GitHub
# App skald-release, its environment and its secrets are set up on GitHub under these names; a
# rename changes them here and in the workflow.

# The workflow that tags the merge of a Release PR and keeps the Release PR open.
release_workflow=.github/workflows/release-plz.yml
# The action its jobs and CI's install release-plz with.
release_install_action=.github/actions/install-release-plz
# The GitHub environment with the App's secrets; only dev deploys from it, only the release-plz
# jobs use it.
release_environment=release-plz
# The App's ID and private key, secrets of that environment.
release_app_id_secret=RELEASE_APP_ID
release_app_key_secret=RELEASE_APP_PRIVATE_KEY
# The release-plz the workflows run and the fixture tests run: release-plz built by
# script/install-release-plz from these crates on crates.io, each checked against the SHA-256 of
# its .crate file, with script/lib/release-plz.patch. It goes into a directory of its own, first on
# PATH here. A change to the patch or to the build bumps the +skald.N: an installed release-plz of
# this version is kept.
release_plz_version=0.4.3+skald.1
release_plz_crates=(
  "release-plz 0.4.3 69bbc232e23322cfaf77a60540e69e2dbd83879e843c4fc9d6ad1ead97d257f6"
  "release_plz_core 0.43.0 6a4ca8c2e8465bf8f282cf057af2972794ed2973f8db0a6658e26923c43de480"
)
release_plz_root=${CARGO_HOME:-$HOME/.cargo}/skald/release-plz-$release_plz_version
PATH=$release_plz_root/bin:$PATH

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
