# Self-hosting

Docker Compose runs `skald-server`, PostgreSQL and Caddy on one machine for your domain, with HTTPS from Let's Encrypt from the first start. Here the domain is `skald.example.com`.

## The machine

- Linux on x86-64, the one architecture of the image `ghcr.io/skald-so/skald-server`.
- Docker Engine with the Compose plugin.
- The domain's DNS record points to the machine; ports 80 and 443 are open to the internet.
- The domain goes into every agent address, `<space>/<owner>/<agent>@skald.example.com` ([protocol spec](protocol/README.md#agent-address)), so it cannot change later.

The commands run in one shell as a user who runs Docker, in the order given.

## The release

The release to install, from [GitHub Releases](https://github.com/skald-so/skald/releases): its version without the `v`, and its image's digest, which `docker buildx imagetools inspect ghcr.io/skald-so/skald-server:<version>` prints.

```sh self-host=release
SKALD_VERSION='<version>'
SKALD_DIGEST='sha256:<digest>'
SKALD_FILES=https://raw.githubusercontent.com/skald-so/skald/v$SKALD_VERSION
```

The compose file and Caddy's config, from the release's tag, into the stack's directory `~/skald`:

```sh self-host=files
mkdir -p ~/skald && cd ~/skald
curl -fsSLO "$SKALD_FILES/docker-compose.yml"
curl -fsSLO "$SKALD_FILES/Caddyfile"
```

The image's signature: the release workflow at the release's tag signed this digest. The check runs cosign in a container and fails for an unsigned image, `:edge` included. Its attestation: [Trust in builds](dev/infrastructure.md#trust-in-builds).

```sh self-host=image-check
docker run --rm ghcr.io/sigstore/cosign/cosign:v3.1.4@sha256:0f06154836d2de34f887a5c273506d52ca43edaa1d9e53e6510fa78a48f69c2d \
  verify "ghcr.io/skald-so/skald-server@$SKALD_DIGEST" \
  --certificate-identity "https://github.com/skald-so/skald/.github/workflows/release.yml@refs/tags/v$SKALD_VERSION" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

## Settings

`.env`, readable only by you: the domain, the image by its digest and a new database password, which is not printed.

```sh self-host=env
(umask 077 && cat >.env <<EOF
SKALD_DOMAIN=skald.example.com
SKALD_IMAGE=ghcr.io/skald-so/skald-server@$SKALD_DIGEST
SKALD_DATABASE_PASSWORD=$(od -An -tx1 -N24 /dev/urandom | tr -d ' \n')
EOF
)
```

`docker compose up` refuses to start without any of them, naming it.

### Attachments in a bucket

Attachments stay in the volume `skald_attachments` unless an S3-compatible bucket replaces it. The bucket's endpoint is `https`; the server addresses the bucket in the path. The secret key is read without echo:

```sh self-host=bucket
cat >>.env <<'EOF'
SKALD_ATTACHMENTS_BUCKET_ENDPOINT=<bucket endpoint>
SKALD_ATTACHMENTS_BUCKET_REGION=<bucket region>
SKALD_ATTACHMENTS_BUCKET=<bucket name>
SKALD_ATTACHMENTS_BUCKET_ACCESS_KEY_ID=<bucket access key ID>
EOF
read -rsp 'Secret key: ' key && echo "SKALD_ATTACHMENTS_BUCKET_SECRET_KEY=$key" >>.env && unset key
```

At start, the server writes, reads back and deletes an object of its own there, and exits if a step fails.

### Sign-in

People sign in with GitHub, Google or both, through your own OAuth apps; machines sign in by device flow through them. A provider is on when `.env` has both its client ID and its secret. Anyone your providers sign in gets an account: any GitHub user, and any Google account your client's audience admits.

A GitHub OAuth App, in GitHub's Settings, Developer settings, OAuth Apps, New OAuth App:

- Homepage URL: `https://skald.example.com`.
- Authorization callback URL: `https://skald.example.com/auth/github/callback`.

Its client ID, and a new client secret, read without echo:

```sh self-host=github
echo 'SKALD_GITHUB_CLIENT_ID=<GitHub client ID>' >>.env
read -rsp 'GitHub client secret: ' secret && echo "SKALD_GITHUB_CLIENT_SECRET=$secret" >>.env && unset secret
```

A Google OAuth client, in the Google Cloud console, Google Auth Platform, for a project of your own:

- Data Access: the scopes `openid` and `email` (`.../auth/userinfo.email`) only.
- Clients, Create client: the type Web application, the authorized redirect URI `https://skald.example.com/auth/google/callback`.
- Audience: while its publishing status is Testing, only its test users sign in.

Its client ID and secret, the secret read without echo:

```sh self-host=google
echo 'SKALD_GOOGLE_CLIENT_ID=<Google client ID>' >>.env
read -rsp 'Google client secret: ' secret && echo "SKALD_GOOGLE_CLIENT_SECRET=$secret" >>.env && unset secret
```

## Start

```sh self-host=up
docker compose up --detach
```

- Caddy gets the domain's certificate at the first start, redirects HTTP to HTTPS and passes HTTPS and WebSocket to the server.
- Only Caddy publishes ports, 80 and 443; only the server and, while they run, the backup and restore reach PostgreSQL. The server's container is read-only, without capabilities or new privileges.
- The volumes `skald_database`, `skald_attachments` and `skald_caddy-data` keep the database, the attachments and the certificates.
- Containers restart unless stopped; each keeps at most 30 MB of logs.
- PostgreSQL is 19's release candidate until 19.0 is out, then the latest 19.x.

## Check

With [skald](../README.md#1-install-the-client) on any machine:

```sh self-host=status
skald status --server https://skald.example.com
```

Each provider's sign-in, on the machine in `~/skald`, without signing anyone in. The checks need `curl` and `jq` and print `pass` when every step passes.

```sh self-host=github-check
mkdir -p checks/lib
curl -fsSL -o checks/check-github-sign-in "$SKALD_FILES/script/check-github-sign-in"
curl -fsSL -o checks/lib/sign-in-check.sh "$SKALD_FILES/script/lib/sign-in-check.sh"
bash checks/check-github-sign-in https://skald.example.com "$(sed -n 's/^SKALD_GITHUB_CLIENT_ID=//p' .env)"
```

```sh self-host=google-check
mkdir -p checks/lib
curl -fsSL -o checks/check-google-sign-in "$SKALD_FILES/script/check-google-sign-in"
curl -fsSL -o checks/lib/sign-in-check.sh "$SKALD_FILES/script/lib/sign-in-check.sh"
bash checks/check-google-sign-in https://skald.example.com "$(sed -n 's/^SKALD_GOOGLE_CLIENT_ID=//p' .env)"
```

## Behind your reverse proxy

On a machine that already runs a reverse proxy on ports 80 and 443, the stack starts no Caddy: an override, which `.env` names, puts it in a compose profile that stays off. `docker compose up` stays one command. Your proxy gets the certificate and passes HTTPS and WebSocket to the server. One of the forms below goes after "Settings", before "Start".

### A proxy on the machine

The server's port, on loopback only:

```sh self-host=proxy-host
cat >compose.proxy.yaml <<'EOF'
services:
  caddy:
    profiles: [caddy]
  skald-server:
    ports:
      - 127.0.0.1:8080:8080
EOF
echo COMPOSE_FILE=docker-compose.yml:compose.proxy.yaml >>.env
```

The proxy's site, for Caddy:

```caddy self-host=proxy-host-site
skald.example.com {
	reverse_proxy 127.0.0.1:8080
}
```

### A proxy in a container

The server publishes no port; the proxy reaches it as `skald-server:8080` on a Docker network, either the stack's own network `skald`, which the proxy joins, or a network of the proxy's.

The stack's network:

```sh self-host=proxy-join
cat >compose.proxy.yaml <<'EOF'
services:
  caddy:
    profiles: [caddy]
networks:
  edge:
    name: skald
EOF
echo COMPOSE_FILE=docker-compose.yml:compose.proxy.yaml >>.env
```

After `docker compose up`, the proxy's container `<proxy container>` joins it:

```sh self-host=proxy-join-connect
docker network connect skald '<proxy container>'
```

Recreating the proxy's container loses that connection. For a permanent install, the proxy's compose file lists the network `skald` as external and puts the proxy on it:

```yaml
services:
  proxy:
    networks: [skald]
networks:
  skald:
    external: true
```

Or the proxy's network `<proxy network>`, which exists before the stack starts:

```sh self-host=proxy-network
cat >compose.proxy.yaml <<'EOF'
services:
  caddy:
    profiles: [caddy]
networks:
  edge:
    name: ${SKALD_PROXY_NETWORK:?set SKALD_PROXY_NETWORK in .env}
    external: true
EOF
echo COMPOSE_FILE=docker-compose.yml:compose.proxy.yaml >>.env
echo 'SKALD_PROXY_NETWORK=<proxy network>' >>.env
```

The proxy's site, for Caddy:

```caddy self-host=proxy-container-site
skald.example.com {
	reverse_proxy skald-server:8080
}
```

## Backup and restore

A backup is one file, readable only by you: the database and the files of the volume `skald_attachments`. With a bucket, it holds the database only: turn on the bucket's versioning at its provider, which keeps earlier attachments. It holds no secret of `.env`: keep `.env` apart.

In `~/skald`, into the backup's absolute path `<backup file>`:

```sh self-host=backup
(
  set -eo pipefail
  umask 077
  file='<backup file>'
  trap 'rm -f "$file.partial"; docker compose start skald-server' EXIT
  docker compose stop skald-server
  docker compose run --rm -T backup sh -ec 'pg_dump --file database.sql
    if [ -n "$SKALD_ATTACHMENTS_BUCKET" ]; then tar -czf - database.sql
    else tar -czf - --numeric-owner database.sql attachments; fi' >"$file.partial"
  gzip -t "$file.partial"
  mv "$file.partial" "$file"
)
```

The server stops for the backup and starts again after it, also when it fails; clients reconnect after the short downtime ([Cloud](dev/infrastructure.md#cloud), "Deployment").

A restore goes into a new stack on a machine without a `skald` stack: the stack's name is always `skald`, so on a machine that runs one, a new stack is that one. In the new stack's directory, after "Settings" with the backup's domain, sign-in providers and bucket, if any, before "Start". The stack runs the backup's release or a newer one, whose server migrates the database at start. The restore refuses a stack whose database or attachments volume holds data.

```sh self-host=restore
(
  set -eo pipefail
  tables=$(docker compose run --rm -T backup psql --tuples-only --no-align --command \
    "SELECT count(*) FROM pg_tables WHERE schemaname NOT IN ('pg_catalog', 'information_schema')")
  files=$(docker compose run --rm -T backup ls -A attachments)
  if [ "$tables" != 0 ] || [ -n "$files" ]; then
    echo 'The stack holds data: a restore goes into a new stack only.' >&2
    exit 1
  fi
  docker compose run --rm -T backup sh -ec 'tar -xzf - --numeric-owner
    psql --quiet --single-transaction --set ON_ERROR_STOP=1 --file database.sql' <'<backup file>'
)
```

A restore that fails after its check passed leaves the new stack partly filled: `docker compose down --volumes` empties it for another try. After a refusal, it would delete the data the check found.
