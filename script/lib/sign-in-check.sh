# What script/check-github-sign-in and script/check-google-sign-in share, sourced by them after
# `set -euo pipefail`: the usage, the server's addresses, the requests, the steps' output, the device
# authorization and the verification page's sign-in start. They make their own HTTPS requests with
# curl, which checks the server's certificate and reads no curl config.

url_parts='^([A-Za-z][A-Za-z0-9+.-]*)://([^/?#]*)(.*)$'
# The machine's client_id, skald, the CLI, and the protocol version its requests state:
# PROTOCOL_VERSION in crates/proto/src/version.rs.
machine=skald
version='Skald-Protocol-Version: 1'

# put <format> <argument>...: printf in a subshell, which has started no child and so gets no
# SIGCHLD. Bash before 4.2, macOS's /bin/bash among them, lets SIGCHLD interrupt a write: a child
# the shell has already waited for can still signal it, and a write that waits for a pipe or a
# terminal then fails with EINTR. The checks print to anything but a file through put or say.
put() { (printf "$@"); }

# say <text>: prints <text> and a newline, as put does.
say() { put '%s\n' "$1"; }

# origin <scheme> <authority>: the origin, in lowercase, without the scheme's default port.
origin() {
  local scheme authority
  scheme=$(tr '[:upper:]' '[:lower:]' <<<"$1")
  authority=$(tr '[:upper:]' '[:lower:]' <<<"$2")
  case $scheme in
    https) authority=${authority%:443} ;;
    http) authority=${authority%:80} ;;
  esac
  put '%s://%s' "$scheme" "$authority"
}

# normal_url <url>: <url> with its origin as origin prints it; fails if <url> is not a URL.
normal_url() {
  [[ $1 =~ $url_parts ]] || return 1
  put '%s%s' "$(origin "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}")" "${BASH_REMATCH[3]}"
}

# address <name> <url>: sets address_origin and address_path, <url>'s origin as origin prints it
# and its path without a trailing slash; exits 2 naming <name> unless <url> is an http or https URL
# without a query, a fragment, spaces or control characters.
address() {
  address_origin='' address_path=''
  if [[ $2 =~ $url_parts && $2 != *[[:space:][:cntrl:]?#]* ]]; then
    address_path=${BASH_REMATCH[3]%/}
    address_origin=$(origin "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}")
  fi
  if [[ $address_origin != http://?* && $address_origin != https://?* ]]; then
    say "$check: the $1 is not an http or https URL" >&2
    exit 2
  fi
}

# check_start <name> <server address> <client ID> [<web address>]: checks the usage and the tools;
# sets check, client_id, server_origin, server_path, server, the server's address without a
# trailing slash, web_origin, web_path, web, the same of the web address, by default the server's,
# and tmp, a directory removed on exit.
check_start() {
  check=$1
  shift
  if [[ $# -lt 2 || $# -gt 3 || -z $2 ]]; then
    say "usage: $0 <server address> <client ID> [<web address>]" >&2
    exit 2
  fi
  local tool
  for tool in curl jq; do
    if ! command -v "$tool" >/dev/null; then
      say "$check: $tool is missing" >&2
      exit 2
    fi
  done
  client_id=$2
  address 'server address' "$1"
  server_origin=$address_origin server_path=$address_path
  server=$server_origin$server_path
  if [[ $# -eq 3 ]]; then
    address 'web address' "$3"
  fi
  web_origin=$address_origin web_path=$address_path
  web=$web_origin$web_path
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/$check.XXXXXX")
  trap 'rm -rf "$tmp"' EXIT
}

# on_address <origin> <path> <url>: prints the part of <url> after the address <origin><path>; fails
# if <url> is not on that address or has spaces or control characters.
on_address() {
  local rest
  [[ $3 != *[[:space:][:cntrl:]]* && $3 =~ $url_parts ]] || return 1
  rest=${BASH_REMATCH[3]}
  [[ $(origin "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}") == "$1" ]] || return 1
  [[ $rest == "$2"/* ]] || return 1
  put '%s' "${rest#"$2"}"
}

# on_web <url>: as on_address, on the web address.
on_web() { on_address "$web_origin" "$web_path" "$1"; }

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
    put '%b\n' "${value//%/\\x}"
  done
}

# device_authorization: POST <server>/auth/device gives a user code, a device code and a
# verification page; the answer in $tmp/device.
device_authorization() {
  local status
  status=$(request device --header "$version" --data client_id="$machine" \
    "$server/auth/device" </dev/null) || {
    say "no answer from $server/auth/device"
    return 1
  }
  [[ $status == 200 ]] || {
    say "HTTP $status from $server/auth/device"
    return 1
  }
  jq -e 'type == "object"' "$tmp/device" >/dev/null 2>&1 || {
    say "the answer is not a JSON object"
    return 1
  }
  jq -e '.user_code | strings | length > 0' "$tmp/device" >/dev/null || {
    say "no user code"
    return 1
  }
  jq -e '.device_code | strings | length > 0' "$tmp/device" >/dev/null || {
    say "no device code"
    return 1
  }
  jq -e '.verification_uri | strings | length > 0' "$tmp/device" >/dev/null || {
    say "no verification page"
    return 1
  }
}

# page_origin <url>: the origin of <url>, the verification page's, as origin prints it, if it has
# only the characters of a host and port and none of the codes in $tmp/device, in any case, with or
# without their hyphens.
page_origin() {
  local scheme authority shown code
  [[ $1 =~ $url_parts ]] || return 1
  scheme=${BASH_REMATCH[1]} authority=${BASH_REMATCH[2]}
  [[ $authority =~ ^[][A-Za-z0-9.:-]+$ ]] || return 1
  shown=$(origin "$scheme" "$authority")
  for code in user_code device_code; do
    code=$(jq -r --arg name "$code" '.[$name] | strings' "$tmp/device" |
      tr '[:upper:]' '[:lower:]')
    [[ -z $code || ${shown//-/} != *"${code//-/}"* ]] || return 1
  done
  put '%s' "$shown"
}

# page_on_web: the verification page of $tmp/device is on the web address; prints its address.
page_on_web() {
  local uri complete off shown
  uri=$(jq -r '.verification_uri | strings' "$tmp/device")
  complete=$(jq -r '.verification_uri_complete | strings' "$tmp/device")
  for off in "$uri" "$complete"; do
    [[ -n $off ]] || continue
    on_web "$off" >/dev/null && continue
    if shown=$(page_origin "$off"); then
      say "the verification page is on $shown, not on $web"
    else
      say "the verification page is not on $web"
    fi
    return 1
  done
  put '%s' "${complete:-$uri}"
}

# sign_in_start <link text> <link> <authorization endpoint> <callback>: the verification page is on
# the web address and, without a session, has a link on the web address whose text matches the
# extended regular expression <link text>, described as <link> in the reasons; the link sends the
# browser to <authorization endpoint>, as normal_url prints it, with the client ID and the callback
# <web><callback>. The address the browser is sent to in $tmp/location, its query in $tmp/query.
sign_in_start() {
  local text=$1 described=$2 authorize=$3 callback=$4
  local page status link start query values
  page=$(page_on_web) || {
    say "$page"
    return 1
  }
  status=$(get page "$page") || {
    say "no answer from the verification page"
    return 1
  }
  [[ $status == 200 ]] || {
    say "HTTP $status from the verification page"
    return 1
  }
  link=$(tr '\n\r\t' '   ' <"$tmp/page" |
    grep -oiE "<a[[:space:]][^>]*>[[:space:]]*$text[[:space:]]*</a>" |
    head -n 1 | grep -oiE '[[:space:]]href="[^"]*"' | head -n 1 |
    sed -e 's/^[^"]*"//' -e 's/"$//' -e 's/&lt;/</g' -e 's/&gt;/>/g' -e 's/&quot;/"/g' \
      -e "s/&#39;/'/g" -e 's/&amp;/\&/g') || true
  [[ -n $link ]] || {
    say "the page has no $described"
    return 1
  }
  # The link's address, resolved against the page's.
  if [[ $link =~ ^[A-Za-z][A-Za-z0-9+.-]*: ]]; then
    start=$link
  elif [[ $link == //* ]]; then
    start=${web_origin%%://*}:$link
  elif [[ $link == /* ]]; then
    start=$web_origin$link
  elif [[ $link == '?'* ]]; then
    start=${page%%[?#]*}$link
  else
    start=${page%%[?#]*}
    start=${start%/*}/$link
  fi
  on_web "$start" >/dev/null || {
    say "the $described leads off $web"
    return 1
  }
  status=$(get start "$start") || {
    say "no answer from the sign-in start"
    return 1
  }
  [[ $status == 30[12378] ]] || {
    say "HTTP $status from the sign-in start, not a redirect"
    return 1
  }
  local location
  location=$(tr -d '\r' <"$tmp/start.headers" | awk 'tolower($1) == "location:"' | tail -n 1 |
    sed -e 's/^[^:]*:[[:space:]]*//' -e 's/[[:space:]]*$//')
  local base=${location%%[?#]*}
  if [[ -z $location || $(normal_url "$base") != "$authorize" ]]; then
    say "the sign-in start does not send the browser to $authorize"
    return 1
  fi
  query=${location#"$base"}
  query=${query#\?}
  query=${query%%#*}
  printf '%s' "$location" >"$tmp/location"
  printf '%s' "$query" >"$tmp/query"
  values=$(param "$query" client_id)
  [[ $values == "$client_id" ]] || {
    say "the sign-in start does not send the client ID $client_id"
    return 1
  }
  values=$(param "$query" redirect_uri)
  [[ -n $values && $(on_web "$values") == "$callback" ]] || {
    say "the sign-in start does not send the callback $web$callback"
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
      say "$(tr _ ' ' <<<"$step"): pass"
    else
      say "$(tr _ ' ' <<<"$step"): fail: ${reason:-it failed}"
      say fail
      exit 1
    fi
  done
  say pass
}
