# The SSH connection of the deploy scripts, script/deploy-staging and script/deploy-production,
# sourced by them. It comes only from three variables, the secrets of the script's GitHub
# environment, named after its <prefix>, such as STAGING:
#   <prefix>_SSH_KEY          the private key, base64
#   <prefix>_SSH_HOST_KEY     the host key pinned for the connection: `<type> <base64>`
#   <prefix>_SSH_DESTINATION  ssh://<account>@<address>:<port>
# Nothing here prints these values: SSH's own messages, which name the address, are off.

# check_ssh_secrets <script> <prefix>: exits with 2 unless the three variables are set and the host
# key and the destination are well-formed.
check_ssh_secrets() {
  local name host_key=$2_SSH_HOST_KEY destination=$2_SSH_DESTINATION
  for name in "$2_SSH_KEY" "$host_key" "$destination"; do
    if [[ -z ${!name:-} ]]; then
      echo "$1: $name is not set" >&2
      exit 2
    fi
  done
  if [[ ! ${!host_key} =~ ^[a-z0-9@.-]+\ [A-Za-z0-9+/]+={0,2}$ ]]; then
    echo "$1: $host_key is not '<type> <base64>'" >&2
    exit 2
  fi
  if [[ ! ${!destination} =~ ^ssh://[^[:space:]]+$ ]]; then
    echo "$1: $destination is not an ssh:// URL" >&2
    exit 2
  fi
}

# ssh_deploy <script> <prefix> <command>...: runs <command> over SSH as the key, the host key pinned,
# whatever SSH config or agent the caller has, with ssh_deploy's input as its own; exits with 2 if the
# key is not a base64 private key, with 1, saying so, if SSH or the command fails. Leaves no file.
ssh_deploy() {
  local script=$1 key=$2_SSH_KEY host_key=$2_SSH_HOST_KEY destination=$2_SSH_DESTINATION tmp status=0
  shift 2
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/$script.XXXXXX")
  # shellcheck disable=SC2064 # $tmp is known now.
  trap "rm -rf '$tmp'" EXIT
  (
    umask 077
    base64 --decode <<<"${!key}" >"$tmp/key" 2>/dev/null
  ) || true
  if [[ $(head -n 1 "$tmp/key") != '-----BEGIN OPENSSH PRIVATE KEY-----' ]]; then
    echo "$script: $key is not a base64 private key" >&2
    exit 2
  fi
  # The pinned host key under an alias: SSH checks it whatever the address and port.
  echo "deploy-host ${!host_key}" >"$tmp/known_hosts"
  ssh -F /dev/null -i "$tmp/key" -o IdentitiesOnly=yes -o IdentityAgent=none -o BatchMode=yes \
    -o StrictHostKeyChecking=yes -o UserKnownHostsFile="$tmp/known_hosts" \
    -o GlobalKnownHostsFile=/dev/null -o HostKeyAlias=deploy-host -o CheckHostIP=no \
    -o UpdateHostKeys=no -o LogLevel=QUIET -o ConnectTimeout=30 -o ServerAliveInterval=15 \
    -- "${!destination}" "$@" || status=$?
  if [[ $status -eq 255 ]]; then
    echo "$script: FAILED: no SSH session: the connection, the pinned host key or the key failed"
    exit 1
  elif [[ $status -ne 0 ]]; then
    echo "$script: FAILED: the deploy exited with $status"
    exit 1
  fi
}
