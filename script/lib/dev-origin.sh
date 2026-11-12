# The scratch repository of the tests of the scripts that act only for the head of dev on origin,
# script/tag-edge-test and script/deploy-staging-test, sourced by them.

# dev_origin <dir>: a bare origin in <dir>/origin.git and its clone in <dir>/clone, isolated from
# the user's git config, with two commits on dev, $older and $head, and another branch named like
# dev at $older: only the head of dev counts.
dev_origin() {
  export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
  export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com
  export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com
  git init --quiet --bare "$1/origin.git"
  git clone --quiet "$1/origin.git" "$1/clone" 2>/dev/null
  git -C "$1/clone" checkout --quiet -b dev
  git -C "$1/clone" commit --quiet --allow-empty --message older
  older=$(git -C "$1/clone" rev-parse HEAD)
  git -C "$1/clone" commit --quiet --allow-empty --message head
  head=$(git -C "$1/clone" rev-parse HEAD)
  git -C "$1/clone" push --quiet origin dev
  git -C "$1/clone" push --quiet origin "$older:refs/heads/feature/dev"
}
