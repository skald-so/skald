# start_server <log> <command>...: runs <command>, a server that prints the port it listens on as the
# first line of its output, in the background with its output in <log>, and waits until it prints
# that line or exits, up to 60 s: node may take that long to start on a slow runner. Sets server_pid
# to its PID and server_port to the port; on a server that did not start, returns 1 and sets
# server_failure to why. Sourced by script/installers-test and script/npm-package-test.
start_server() {
  local log=$1 tick
  shift
  "$@" >"$log" &
  server_pid=$!
  server_port=
  server_failure='it printed no port in 60 s'
  for tick in $(seq 600); do
    if [[ $(wc -l <"$log") -ge 1 ]]; then
      server_port=$(head -n 1 "$log" | tr -d '\r')
      [[ $server_port =~ ^[0-9]+$ ]] && return 0
      server_failure="it printed '$server_port', not its port"
      return 1
    fi
    if ! kill -0 "$server_pid" 2>/dev/null; then
      server_failure="it exited after $((tick / 10)) s"
      return 1
    fi
    sleep 0.1
  done
  return 1
}
