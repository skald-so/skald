# The part's own steps between the self-host guide's, on the clean machine of script/self-host-test.
# Sourced by the script of each scenario, which sets $self_host_scenario first; the values come from
# /self-host/params. The scenario's files go to $run. Everything here runs on the machine.
# shellcheck shell=bash
# shellcheck source=/dev/null
source /self-host/params
run=/self-host/run/$self_host_scenario
jq=/opt/self-host-test/bin/jq
self_host_path=$PATH

# self_host_step <step>: marks the start of the guide's step in the scenario's output.
self_host_step() { echo "self-host-test: step $1"; }

# self_host_release: the release values the part sets after the guide's step `release`.
self_host_release() {
  SKALD_VERSION=$self_host_version
  SKALD_DIGEST=$self_host_digest
  SKALD_FILES=$self_host_files
}

# self_host_before_up: after the guide's settings, before `docker compose up`: records the stack's
# directory and the mode of .env; puts this commit's image in place of the release's; in the main
# scenario, tries `docker compose up` without each value of .env; then points Docker Compose to the
# override that stands in for the internet.
self_host_before_up() {
  pwd >"$run/dir"
  stat -c %a .env >"$run/env-mode"
  if ! grep -qF "ghcr.io/skald-so/skald-server@$self_host_digest" .env; then
    echo "self-host-test: .env does not name the release's image by its digest," \
      "ghcr.io/skald-so/skald-server@$self_host_digest" >&2
    return 1
  fi
  sed -i "s|ghcr.io/skald-so/skald-server@$self_host_digest|$self_host_image|g" .env
  cp -p .env "$run/env"
  if [[ $self_host_scenario == main ]]; then
    self_host_missing_values
  fi
  self_host_override
}

# self_host_missing_values: for each value of .env, `docker compose up` without it; writes
# `<name>\t<exit code>\t<containers it left>` lines to $run/missing and its output to
# $run/missing-<name>, and removes what it started.
self_host_missing_values() {
  local name status output left
  : >"$run/missing"
  for name in $(sed -n 's/^\([A-Za-z_][A-Za-z0-9_]*\)=.*/\1/p' .env); do
    sed -i "/^$name=/d" .env
    status=0
    output=$(docker compose up --detach </dev/null 2>&1) || status=$?
    printf '%s\n' "$output" >"$run/missing-$name"
    left=$(docker ps --all --quiet --filter "label=com.docker.compose.project.working_dir=$PWD")
    printf '%s\t%s\t%s\n' "$name" "$status" "$(wc -w <<<"$left" | tr -d ' ')" >>"$run/missing"
    cp -p "$run/env" .env
    if [[ -n $left ]]; then
      docker compose down --volumes --remove-orphans </dev/null >/dev/null 2>&1 || true
      # $left unquoted: one container ID per word.
      docker rm --force --volumes $left >/dev/null 2>&1 || true
    fi
  done
}

# self_host_override: exports COMPOSE_FILE: the stack's compose files and, last, an override that
# only stands in for the internet. It adds to the server's and Caddy's services, found by their
# images, the test CA in SSL_CERT_FILE and the network of the stand-ins, keeping their own networks;
# to the server's, the addresses of the GitHub and Google stand-ins.
self_host_override() {
  local files name
  files=$(sed -n 's/^COMPOSE_FILE=//p' .env | tail -n 1)
  files=${files#[\"\']}
  files=${files%[\"\']}
  if [[ -z $files ]]; then
    for name in compose.yaml compose.yml docker-compose.yaml docker-compose.yml; do
      if [[ -f $name ]]; then
        files=$name
        break
      fi
    done
    for name in compose.override.yaml compose.override.yml docker-compose.override.yaml \
      docker-compose.override.yml; do
      if [[ -f $name ]]; then
        files+=:$name
        break
      fi
    done
  fi
  docker compose config --format json | "$jq" --arg image "$self_host_image" \
    --arg ca "$self_host_ca" --arg internet "$self_host_internet" \
    --arg github "$self_host_github_url" --arg google "$self_host_google_issuer" '
    [.services | to_entries[]
      | select(.value.image == $image or ((.value.image // "") | test("(^|/)caddy[:@]")))]
    | {
        services: map({
          key,
          value: {
            environment: ({SSL_CERT_FILE: "/etc/self-host-test/ca.pem"}
              + if .value.image == $image then
                  {SKALD_GITHUB_URL: $github, SKALD_GITHUB_API_URL: $github,
                    SKALD_GOOGLE_ISSUER: $google}
                else {} end),
            volumes: [{type: "bind", source: $ca, target: "/etc/self-host-test/ca.pem", read_only: true}],
            networks: ((.value.networks // {default: null}) + {"self-host-test-internet": null})
          }
        }) | from_entries,
        networks: {"self-host-test-internet": {external: true, name: $internet}}
      }' >"$run/override.json"
  export COMPOSE_FILE=$files:$run/override.json
  printf '%s\n' "$COMPOSE_FILE" >"$run/compose-file"
}

# self_host_wait_https: waits up to 180 s until the domain's /health answers over HTTPS with a
# certificate of the local ACME server.
self_host_wait_https() {
  for _ in $(seq 180); do
    if curl --silent --fail --output /dev/null --cacert "$self_host_acme_root" \
      "https://$self_host_domain/health"; then
      return 0
    fi
    sleep 1
  done
  echo "self-host-test: https://$self_host_domain/health did not answer within 180 s" >&2
  return 1
}

# self_host_trust_acme, self_host_trust_system: what skald trusts in the guide's `status` step:
# only the local ACME server's root, then again the system's roots, without the stand-ins' addresses
# of self_host_trust_checks.
self_host_trust_acme() { export SSL_CERT_FILE=$self_host_acme_root; }
self_host_trust_system() {
  unset SSL_CERT_FILE GITHUB_URL GOOGLE_ISSUER
  PATH=$self_host_path
}

# self_host_trust_checks: what the guide's sign-in checks reach: the GitHub and Google stand-ins in
# GITHUB_URL and GOOGLE_ISSUER, and the roots they trust, the local ACME server's and the test CA.
# The checks need jq besides curl, as on the admin's own machine: it joins PATH.
self_host_trust_checks() {
  cat "$self_host_acme_root" "$self_host_ca" >"$run/trust.pem"
  export SSL_CERT_FILE=$run/trust.pem GITHUB_URL=$self_host_github_url \
    GOOGLE_ISSUER=$self_host_google_issuer PATH=/opt/self-host-test/checks:$PATH
}

# proxy_caddyfile <ACME directory> <ACME server's TLS root> [<storage>]: the admin's proxy's
# Caddyfile: the guide's site block, its certificate from the local ACME server.
proxy_caddyfile() {
  printf '{\n\tadmin off\n\tacme_ca %s\n\tacme_ca_root %s\n' "$1" "$2"
  if [[ -n ${3:-} ]]; then
    printf '\tstorage file_system %s\n' "$3"
  fi
  printf '}\n\n'
  cat "$run/site.caddy"
}

# self_host_proxy_on_host: the admin's proxy on the machine itself, a Caddy process with the
# guide's site block; its PID goes to $run/proxy/pid.
self_host_proxy_on_host() {
  mkdir -p "$run/proxy/data"
  proxy_caddyfile https://127.0.0.1:14000/dir "$self_host_ca" "$run/proxy/data" >"$run/proxy/Caddyfile"
  setsid /opt/self-host-test/bin/caddy run --config "$run/proxy/Caddyfile" --adapter caddyfile \
    </dev/null >"$run/proxy/log" 2>&1 &
  echo $! >"$run/proxy/pid"
}

# self_host_proxy_in_container [network]: the admin's proxy in a container, Caddy with the guide's
# site block, publishing ports 80 and 443; with `network`, on a network of its own that exists
# before the stack.
self_host_proxy_in_container() {
  local network=()
  mkdir -p "$run/proxy"
  proxy_caddyfile https://host.docker.internal:14000/dir /etc/self-host-test/ca.pem \
    >"$run/proxy/Caddyfile"
  if [[ ${1:-} == network ]]; then
    docker network create "$self_host_proxy_network" >/dev/null
    network=(--network "$self_host_proxy_network")
  fi
  docker run --detach --name "$self_host_proxy_container" "${network[@]}" \
    --publish 80:80 --publish 443:443 --add-host host.docker.internal:host-gateway \
    --volume "$run/proxy/Caddyfile:/etc/caddy/Caddyfile:ro" \
    --volume "$self_host_ca:/etc/self-host-test/ca.pem:ro" "$self_host_caddy_image" >/dev/null
}

# self_host_resume <scenario>: continues in the stack of an earlier scenario: its directory and its
# compose files.
self_host_resume() {
  cd "$(cat "/self-host/run/$1/dir")"
  COMPOSE_FILE=$(cat "/self-host/run/$1/compose-file")
  export COMPOSE_FILE
}

# backup_finds <backup> <env> [<label>=<text>]...: what the backup holds, as lines: `checked <label>`
# for each <text> and each secret value of the .env <env>, a value whose name has PASSWORD or SECRET,
# labelled by that name; `holds <label>` for each the backup holds. It looks in the backup's bytes
# and, when it is a tar archive, compressed or not, in its members' bytes, gunzipped when they are
# gzip-compressed.
backup_finds() {
  local backup=$1 env=$2 dir pair name value label text member
  shift 2
  dir=$(mktemp -d)
  mkdir -p "$dir/content/members"
  cp "$backup" "$dir/content/backup"
  if gzip -t "$backup" 2>/dev/null; then gzip -dc "$backup" >"$dir/content/backup.gunzipped"; fi
  tar -xf "$backup" -C "$dir/content/members" 2>/dev/null || true
  while IFS= read -r -d '' member; do
    if gzip -t "$member" 2>/dev/null; then gzip -dc "$member" >"$member.gunzipped"; fi
  done < <(find "$dir/content/members" -type f -print0)
  {
    for pair in "$@"; do printf '%s\t%s\n' "${pair%%=*}" "${pair#*=}"; done
    while IFS='=' read -r name value; do
      value=${value#[\"\']}
      value=${value%[\"\']}
      if [[ $name =~ (PASSWORD|SECRET) && -n $value ]]; then printf '%s\t%s\n' "$name" "$value"; fi
    done < <(grep -E '^[A-Za-z_][A-Za-z0-9_]*=' "$env")
  } >"$dir/needles"
  while IFS=$'\t' read -r label text; do
    echo "checked $label"
    if grep -rqaF -- "$text" "$dir/content"; then echo "holds $label"; fi
  done <"$dir/needles"
  rm -rf "$dir"
}
