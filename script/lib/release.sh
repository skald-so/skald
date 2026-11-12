# The names the release-plz workflow and its tests share, sourced by script/release-plz-test,
# script/release-plz-workflow-test and the release tag tests. The GitHub App skald-release, its
# environment and its secrets are set up on GitHub under these names; a rename changes them here and
# in the workflow.

# The workflow that tags the merge of a Release PR and keeps the Release PR open.
release_workflow=.github/workflows/release-plz.yml
# The GitHub environment with the App's secrets; only dev deploys from it, only the release-plz
# jobs use it.
release_environment=release-plz
# The App's ID and private key, secrets of that environment.
release_app_id_secret=RELEASE_APP_ID
release_app_key_secret=RELEASE_APP_PRIVATE_KEY
# The release-plz the workflow installs and the fixture tests run.
release_plz_version=0.4.3

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
