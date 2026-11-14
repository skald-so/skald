# The part's own steps between the self-host guide's, on the clean machine of script/self-host-test.
# Sourced by the script of each scenario, which sets $self_host_scenario first; the values come from
# /self-host/params. The scenario's files go to $run. Everything here runs on the machine.
# shellcheck shell=bash
# shellcheck source=/dev/null
source /self-host/params
run=/self-host/run/$self_host_scenario
jq=/opt/self-host-test/bin/jq

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
# images, the test CA in SSL_CERT_FILE and the network of the stand-ins, keeping their own networks.
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
    --arg ca "$self_host_ca" --arg internet "$self_host_internet" '
    [.services | to_entries[]
      | select(.value.image == $image or ((.value.image // "") | test("(^|/)caddy[:@]")))]
    | {
        services: map({
          key,
          value: {
            environment: {SSL_CERT_FILE: "/etc/self-host-test/ca.pem"},
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
# only the local ACME server's root, then again the system's roots.
self_host_trust_acme() { export SSL_CERT_FILE=$self_host_acme_root; }
self_host_trust_system() { unset SSL_CERT_FILE; }

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
