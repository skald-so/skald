# What script/check-github-sign-in and script/check-google-sign-in share, sourced by them after
# `set -euo pipefail`: the usage, the server's address, the requests, the steps' output, the device
# authorization and the verification page's sign-in start. They make their own HTTPS requests with
# curl, which checks the server's certificate and reads no curl config.

url_parts='^([A-Za-z][A-Za-z0-9+.-]*)://([^/?#]*)(.*)$'
# The machine's client_id, skald, the CLI, and the protocol version its requests state:
# PROTOCOL_VERSION in crates/proto/src/version.rs.
machine=skald
version='Skald-Protocol-Version: 1'

# origin <scheme> <authority>: the origin, in lowercase, without the scheme's default port.
origin() {
  local scheme authority
  scheme=$(tr '[:upper:]' '[:lower:]' <<<"$1")
  authority=$(tr '[:upper:]' '[:lower:]' <<<"$2")
  case $scheme in
    https) authority=${authority%:443} ;;
    http) authority=${authority%:80} ;;
  esac
  printf '%s://%s' "$scheme" "$authority"
}

# normal_url <url>: <url> with its origin as origin prints it; fails if <url> is not a URL.
normal_url() {
  [[ $1 =~ $url_parts ]] || return 1
  printf '%s%s' "$(origin "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}")" "${BASH_REMATCH[3]}"
}

# check_start <name> <server address> <client ID>: checks the usage and the tools; sets check,
# client_id, server_origin, server_path, server, the server's address without a trailing slash,
# and tmp, a directory removed on exit.
check_start() {
  check=$1
  shift
  if [[ $# -ne 2 || -z $2 ]]; then
    echo "usage: $0 <server address> <client ID>" >&2
    exit 2
  fi
  local tool
  for tool in curl jq; do
    if ! command -v "$tool" >/dev/null; then
      echo "$check: $tool is missing" >&2
      exit 2
    fi
  done
  client_id=$2
  if [[ $1 =~ $url_parts && $1 != *[[:space:][:cntrl:]?#]* ]]; then
    server_path=${BASH_REMATCH[3]%/}
    server_origin=$(origin "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}")
  fi
  server=${server_origin:-}${server_path:-}
  if [[ ${server_origin:-} != http://?* && ${server_origin:-} != https://?* ]]; then
    echo "$check: the server address is not an http or https URL" >&2
    exit 2
  fi
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/$check.XXXXXX")
  trap 'rm -rf "$tmp"' EXIT
}

# on_server <url>: prints the part of <url> after the server's address; fails if <url> is not on
# the server or has spaces or control characters.
on_server() {
  local rest
  [[ $1 != *[[:space:][:cntrl:]]* && $1 =~ $url_parts ]] || return 1
  rest=${BASH_REMATCH[3]}
  [[ $(origin "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}") == "$server_origin" ]] || return 1
  [[ $rest == "$server_path"/* ]] || return 1
  printf '%s' "${rest#"$server_path"}"
}

# request <name> <curl option>...: requests with curl, the URL among the options, the answer's body
# in $tmp/<name>, its headers in $tmp/<name>.headers; prints the HTTP status. Fails without an
# answer. Reads more curl options from stdin, in curl's config format.
request() {
  local name=$1
  shift
  : >"$tmp/$name"
  : >"$tmp/$name.headers"
  curl --disable --silent --max-time 30 --proto =http,https --proto-redir =http,https --config - \
    --output "$tmp/$name" --dump-header "$tmp/$name.headers" --write-out '%{http_code}' \
    "$@" 2>/dev/null
}

# get <name> <url> [<curl option>...]: as request, a GET of <url>, which stays out of the process
# list: it can hold a code.
get() {
  local name=$1 url=${2//\\/\\\\}
  shift 2
  url=${url//\"/\\\"}
  request "$name" "$@" <<<"url = \"$url\""
}

# param <query> <name>: the decoded values of the parameter <name> in <query>, one per line.
param() {
  local pair value
  local IFS='&'
  for pair in $1; do
    [[ $pair == "$2="* ]] || continue
    value=${pair#*=}
    value=${value//+/ }
    value=${value//\\/\\\\}
    printf '%b\n' "${value//%/\\x}"
  done
}

# device_authorization: POST <server>/auth/device gives a user code, a device code and a
# verification page on that server; the answer in $tmp/device, the page's address in $tmp/page-url.
device_authorization() {
  local status
  status=$(request device --header "$version" --data client_id="$machine" \
    "$server/auth/device" </dev/null) || {
    echo "no answer from $server/auth/device"
    return 1
  }
  [[ $status == 200 ]] || {
    echo "HTTP $status from $server/auth/device"
    return 1
  }
  jq -e 'type == "object"' "$tmp/device" >/dev/null 2>&1 || {
    echo "the answer is not a JSON object"
    return 1
  }
  jq -e '.user_code | strings | length > 0' "$tmp/device" >/dev/null || {
    echo "no user code"
    return 1
  }
  jq -e '.device_code | strings | length > 0' "$tmp/device" >/dev/null || {
    echo "no device code"
    return 1
  }
  local uri complete
  uri=$(jq -r '.verification_uri | strings' "$tmp/device")
  complete=$(jq -r '.verification_uri_complete | strings' "$tmp/device")
  [[ -n $uri ]] || {
    echo "no verification page"
    return 1
  }
  on_server "$uri" >/dev/null && { [[ -z $complete ]] || on_server "$complete" >/dev/null; } || {
    echo "the verification page is not on $server"
    return 1
  }
  printf '%s' "${complete:-$uri}" >"$tmp/page-url"
}

# sign_in_start <link text> <link> <authorization endpoint> <callback>: the verification page,
# without a session, has a link on the server whose text matches the extended regular expression
# <link text>, described as <link> in the reasons; the link sends the browser to <authorization
# endpoint>, as normal_url prints it, with the client ID and the callback <server><callback>. The
# address the browser is sent to in $tmp/location, its query in $tmp/query.
sign_in_start() {
  local text=$1 described=$2 authorize=$3 callback=$4
  local page status link start query values
  page=$(cat "$tmp/page-url")
  status=$(get page "$page") || {
    echo "no answer from the verification page"
    return 1
  }
  [[ $status == 200 ]] || {
    echo "HTTP $status from the verification page"
    return 1
  }
  link=$(tr '\n\r\t' '   ' <"$tmp/page" |
    grep -oiE "<a[[:space:]][^>]*>[[:space:]]*$text[[:space:]]*</a>" |
    head -n 1 | grep -oiE '[[:space:]]href="[^"]*"' | head -n 1 |
    sed -e 's/^[^"]*"//' -e 's/"$//' -e 's/&lt;/</g' -e 's/&gt;/>/g' -e 's/&quot;/"/g' \
      -e "s/&#39;/'/g" -e 's/&amp;/\&/g') || true
  [[ -n $link ]] || {
    echo "the page has no $described"
    return 1
  }
  # The link's address, resolved against the page's.
  if [[ $link =~ ^[A-Za-z][A-Za-z0-9+.-]*: ]]; then
    start=$link
  elif [[ $link == //* ]]; then
    start=${server_origin%%://*}:$link
  elif [[ $link == /* ]]; then
    start=$server_origin$link
  elif [[ $link == '?'* ]]; then
    start=${page%%[?#]*}$link
  else
    start=${page%%[?#]*}
    start=${start%/*}/$link
  fi
  on_server "$start" >/dev/null || {
    echo "the $described leads off $server"
    return 1
  }
  status=$(get start "$start") || {
    echo "no answer from the sign-in start"
    return 1
  }
  [[ $status == 30[12378] ]] || {
    echo "HTTP $status from the sign-in start, not a redirect"
    return 1
  }
  local location
  location=$(tr -d '\r' <"$tmp/start.headers" | awk 'tolower($1) == "location:"' | tail -n 1 |
    sed -e 's/^[^:]*:[[:space:]]*//' -e 's/[[:space:]]*$//')
  local base=${location%%[?#]*}
  if [[ -z $location || $(normal_url "$base") != "$authorize" ]]; then
    echo "the sign-in start does not send the browser to $authorize"
    return 1
  fi
  query=${location#"$base"}
  query=${query#\?}
  query=${query%%#*}
  printf '%s' "$location" >"$tmp/location"
  printf '%s' "$query" >"$tmp/query"
  values=$(param "$query" client_id)
  [[ $values == "$client_id" ]] || {
    echo "the sign-in start does not send the client ID $client_id"
    return 1
  }
  values=$(param "$query" redirect_uri)
  [[ -n $values && $(on_server "$values") == "$callback" ]] || {
    echo "the sign-in start does not send the callback $server$callback"
    return 1
  }
}

# run_steps <check function>...: runs each check function, the step named after it, only after the
# one before passed; prints a line `<step>: pass` or `<step>: fail: <reason>` for each, the reason
# what the function printed, then `pass` or `fail`. Exits 0 only when every step passed, 1 if not.
run_steps() {
  local step reason
  for step in "$@"; do
    if reason=$("$step"); then
      echo "$(tr _ ' ' <<<"$step"): pass"
    else
      echo "$(tr _ ' ' <<<"$step"): fail: ${reason:-it failed}"
      echo fail
      exit 1
    fi
  done
  echo pass
}
