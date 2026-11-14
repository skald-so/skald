# The GitHub API calls of script/release-pr, script/release-pr-merged and script/release-tag,
# sourced by them. Each defines error <message>, which exits.

# require_vars <name>...: calls error unless each variable <name> is set and not empty.
require_vars() {
  local name
  for name in "$@"; do
    [[ -n ${!name:-} ]] || error "$name is not set"
  done
}

# github_api <method> <url> [<JSON file>]: a call of the GitHub API at <url> with the token
# $GH_TOKEN and the body <JSON file>, if any; prints the response. A GET is retried up to 3 times.
# False when the call fails or GitHub answers with an error.
github_api() {
  local args=(--fail-with-body --silent --show-error --max-time 60 --request "$1"
    --header "Authorization: Bearer $GH_TOKEN" --header 'Accept: application/vnd.github+json'
    --header 'X-GitHub-Api-Version: 2022-11-28')
  [[ $1 != GET ]] || args+=(--retry 3)
  [[ -z ${3:-} ]] || args+=(--header 'Content-Type: application/json' --data-binary "@$3")
  curl "${args[@]}" "$2"
}
