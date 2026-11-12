# The names the Release PR workflow and its tests share, sourced by script/release-plz-test and
# script/release-plz-workflow-test. The GitHub App skald-release, its environment and its secrets
# are set up on GitHub under these names; a rename changes them here and in the workflow.

# The workflow that keeps the Release PR open.
release_workflow=.github/workflows/release-plz.yml
# The GitHub environment with the App's secrets; only dev deploys from it, only the release-plz
# jobs use it.
release_environment=release-plz
# The App's ID and private key, secrets of that environment.
release_app_id_secret=RELEASE_APP_ID
release_app_key_secret=RELEASE_APP_PRIVATE_KEY
# The release-plz the workflow installs and the fixture tests run.
release_plz_version=0.4.3
