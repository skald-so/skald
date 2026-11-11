# PostgreSQL 19 in a container for the integration tests. Sourced by script/test, script/mutants and
# script/image-test; the caller removes the container with postgres_stop, e.g. in its EXIT trap.

# postgres_start: starts the container and sets $postgres to its ID.
# 19rc1 until PostgreSQL 19.0 is released. The data lives in a tmpfs, so the container leaves no
# volume behind; it fails if the container still gets one.
postgres_start() {
  postgres=$(docker run --detach --rm --tmpfs /var/lib/postgresql --env POSTGRES_PASSWORD=skald \
    --publish 127.0.0.1::5432 postgres:19rc1 -c fsync=off)
  if [[ -n $(docker inspect --format '{{range .Mounts}}{{.Name}}{{end}}' "$postgres") ]]; then
    echo "script/${0##*/}: the PostgreSQL container has a volume" >&2
    exit 1
  fi
}

# postgres_wait: waits until PostgreSQL accepts connections, then exports SKALD_TEST_DATABASE_URL.
postgres_wait() {
  # The image's init scripts run on a server without TCP; the final server listens on TCP.
  for _ in $(seq 120); do
    docker exec "$postgres" pg_isready --quiet --host 127.0.0.1 --username postgres && break
    sleep 0.5
  done
  if ! docker exec "$postgres" pg_isready --quiet --host 127.0.0.1 --username postgres; then
    echo "script/${0##*/}: PostgreSQL did not start" >&2
    docker logs "$postgres" >&2
    exit 1
  fi
  SKALD_TEST_DATABASE_URL="postgres://postgres:skald@$(docker port "$postgres" 5432/tcp | head -n 1)"
  export SKALD_TEST_DATABASE_URL
}

# postgres_stop: removes the container and its volumes, if it was started.
postgres_stop() {
  if [[ -n ${postgres:-} ]]; then
    docker rm --force --volumes "$postgres" >/dev/null
  fi
}
