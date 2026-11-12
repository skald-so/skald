# The checks of the scripts that act for a commit of dev and its skald-server image, script/tag-edge
# and script/deploy-staging, sourced by them. Each acts only while that commit is the head of dev on
# origin: a run for an older commit, such as one that waited for another or a rerun, changes
# nothing, so what it moves never moves back. .github/workflows/edge.yml runs one at a time, so no
# newer run acts between the check and the act.

# check_commit_and_digest <script> <commit> <digest>: exits with 2 unless <commit> is a full commit
# hash and <digest> an image digest.
check_commit_and_digest() {
  if [[ ! $2 =~ ^[0-9a-f]{40}$ ]]; then
    echo "$1: '$2' is not a full commit hash" >&2
    exit 2
  fi
  if [[ ! $3 =~ ^sha256:[0-9a-f]{64}$ ]]; then
    echo "$1: '$3' is not an image digest" >&2
    exit 2
  fi
}

# exit_unless_dev_head <script> <commit> <what stays>: exits with 0, saying <what stays> stays,
# unless <commit> is the head of dev on origin; fails if origin has no dev.
exit_unless_dev_head() {
  local head
  head=$(git ls-remote --exit-code origin refs/heads/dev | cut -f 1)
  if [[ $head != "$2" ]]; then
    echo "$1: dev has moved on to $head; $3 stays"
    exit 0
  fi
}
