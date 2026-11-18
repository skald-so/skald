# Self-hosting

Docker Compose runs `skald-server`, PostgreSQL and Caddy on one machine for your domain, with HTTPS from Let's Encrypt from the first start; Caddy also serves the web app there. Here the domain is `skald.example.com`.

## The machine

- Linux on x86-64, the one architecture of the image `ghcr.io/skald-so/skald-server`.
- Docker Engine with the Compose plugin 2.24.4 or later.
- The domain's DNS record points to the machine; ports 80 and 443 are open to the internet.
- The domain goes into every agent address, `<space>/<owner>/<agent>@skald.example.com` ([protocol spec](protocol/README.md#agent-address)), so it cannot change later.

The commands run in one shell as a user who runs Docker, in the order given. A command that would read standard input without needing it reads `/dev/null` instead, so commands pasted after it still run.

## The release

The release to install, from [GitHub Releases](https://github.com/skald-so/skald/releases): its version without the `v`, and its image's digest, which `docker buildx imagetools inspect ghcr.io/skald-so/skald-server:<version>` prints.

```sh self-host=release
SKALD_VERSION='<version>'
SKALD_DIGEST='sha256:<digest>'
SKALD_FILES=https://raw.githubusercontent.com/skald-so/skald/v$SKALD_VERSION
SKALD_WEB=https://github.com/skald-so/skald/releases/download/v$SKALD_VERSION
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

### The web app

The release's web app: its archive, the archive's checksum and the bundle of its attestation, into `~/skald`:

```sh self-host=web-files
curl -fsSLO "$SKALD_WEB/skald-web.tar.gz"
curl -fsSLO "$SKALD_WEB/skald-web.tar.gz.sha256"
curl -fsSLO "$SKALD_WEB/skald-web.tar.gz.sigstore.json"
```

The archive's checksum, then its attestation by the release workflow at the release's tag, with cosign in a container ([Trust in builds](dev/infrastructure.md#trust-in-builds)). If the check fails, stop: nothing is unpacked yet.

```sh self-host=web-check
sha256sum --check --strict skald-web.tar.gz.sha256
docker run --rm --volume "$PWD/skald-web.tar.gz:/check/skald-web.tar.gz:ro" \
  --volume "$PWD/skald-web.tar.gz.sigstore.json:/check/skald-web.tar.gz.sigstore.json:ro" \
  ghcr.io/sigstore/cosign/cosign:v3.1.4@sha256:0f06154836d2de34f887a5c273506d52ca43edaa1d9e53e6510fa78a48f69c2d \
  verify-blob-attestation /check/skald-web.tar.gz --bundle /check/skald-web.tar.gz.sigstore.json \
  --type slsaprovenance1 \
  --certificate-identity "https://github.com/skald-so/skald/.github/workflows/release.yml@refs/tags/v$SKALD_VERSION" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

The checked web app in place, in `~/skald/web`, which Caddy serves; the one before it is removed:

```sh self-host=web
(
  set -e
  rm -rf web.new web.old
  mkdir web.new
  tar -xzf skald-web.tar.gz -C web.new --no-same-owner
  if [ -d web ]; then mv web web.old; fi
  mv web.new web
  rm -rf web.old
)
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

The server sends no error reports: the stack leaves `SKALD_SENTRY_DSN` unset.

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

People sign in with GitHub, Google or both, through your own OAuth apps; machines sign in by device flow through them. A provider is on when `.env` has both its client ID and its secret; with only one of them, the server does not start and names the other. Who gets an account: [Sign-up](#sign-up). Browsers and machines both use your domain: leave the server setting `SKALD_WEB_URL` unset.

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

### Sign-up

Sign-up is open by default: anyone your providers sign in gets an account, any GitHub user and any Google account your client's audience admits. The server's log then warns of it at every start.

Closed, only the listed people create accounts; existing accounts sign in as before. Each list is comma-separated, compared without case, and may stay empty:

- GitHub logins. GitHub gives no verified email, so a login counts, never an email: whoever holds a listed login when signing up gets the account, also after a rename frees it.
- Google emails, once Google has verified them.
- Google domains: the Workspace or Cloud organization of the account, never an email's domain.

```sh self-host=sign-up
cat >>.env <<'EOF'
SKALD_SIGN_UP=closed
SKALD_SIGN_UP_GITHUB_LOGINS=<GitHub logins>
SKALD_SIGN_UP_GOOGLE_EMAILS=<Google emails>
SKALD_SIGN_UP_GOOGLE_DOMAINS=<Google domains>
EOF
```

A refused person sees that sign-up is closed in the browser, and `skald sign-in` on their machine says so and stops.

## Start

```sh self-host=up
docker compose up --detach
```

- Caddy gets the domain's certificate at the first start and redirects HTTP to HTTPS. It passes the paths of the [OpenAPI spec](../api/openapi.json), HTTPS and WebSocket, to the server, and serves the web app at every other path, an unknown one with its index page. The web app's answers admit only its own origin and no framing (`Content-Security-Policy`).
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

On a machine that already runs a reverse proxy on ports 80 and 443, the stack's Caddy takes neither: an override, which `.env` names, has it serve plain HTTP on port 8080, without a certificate. `docker compose up` stays one command. Your proxy gets the certificate and passes HTTPS and WebSocket to the stack's Caddy, which serves the web app and passes the API to the server as in [Start](#start); your proxy needs no file of the web app. One of the forms below goes after "Settings", before "Start".

### A proxy on the machine

Caddy's port, on loopback only:

```sh self-host=proxy-host
cat >compose.proxy.yaml <<'EOF'
services:
  caddy:
    environment:
      - SKALD_SITE=http://:8080
    ports: !override
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

The stack publishes no port; the proxy reaches its Caddy as `skald-caddy:8080` on a Docker network, either the stack's own network `skald`, which the proxy joins, or a network of the proxy's.

The stack's network:

```sh self-host=proxy-join
cat >compose.proxy.yaml <<'EOF'
services:
  caddy:
    environment:
      - SKALD_SITE=http://:8080
    ports: !reset []
    networks:
      edge:
        aliases: [skald-caddy]
networks:
  edge:
    name: skald
EOF
echo COMPOSE_FILE=docker-compose.yml:compose.proxy.yaml >>.env
```

After `docker compose up`, the proxy's container `<proxy container>` joins it. `--gw-priority -1` keeps the proxy's default route and published ports on its own networks: without it, a network whose name sorts first, such as `skald`, takes them. It needs Docker Engine 28 or later; `gw_priority` in a compose file, below, Docker Compose 2.33.1 or later.

```sh self-host=proxy-join-connect
docker network connect --gw-priority -1 skald '<proxy container>'
```

Recreating the proxy's container loses that connection. For a permanent install, the proxy's compose file lists the network `skald` as external and puts the proxy on it, with the same priority, next to its networks so far:

```yaml
services:
  proxy:
    networks:
      default: {}
      skald:
        gw_priority: -1
networks:
  skald:
    external: true
```

Or the proxy's network `<proxy network>`, which exists before the stack starts:

```sh self-host=proxy-network
cat >compose.proxy.yaml <<'EOF'
services:
  caddy:
    environment:
      - SKALD_SITE=http://:8080
    ports: !reset []
    networks:
      edge:
        aliases: [skald-caddy]
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
	reverse_proxy skald-caddy:8080
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
    else tar -czf - --numeric-owner database.sql attachments; fi' </dev/null >"$file.partial"
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
    "SELECT count(*) FROM pg_tables WHERE schemaname NOT IN ('pg_catalog', 'information_schema')" </dev/null)
  files=$(docker compose run --rm -T backup ls -A attachments </dev/null)
  if [ "$tables" != 0 ] || [ -n "$files" ]; then
    echo 'The stack holds data: a restore goes into a new stack only.' >&2
    exit 1
  fi
  docker compose run --rm -T backup sh -ec 'tar -xzf - --numeric-owner
    psql --quiet --single-transaction --set ON_ERROR_STOP=1 --file database.sql' <'<backup file>'
)
```

A restore that fails after its check passed leaves the new stack partly filled: `docker compose down --volumes` empties it for another try. After a refusal, it would delete the data the check found.

## Upgrade

To a new release, in `~/skald`:

1. [The release](#the-release) with the new release's values, and its image's signature.
2. [The web app](#the-web-app)'s files and their check.
3. A [backup](#backup-and-restore) into a new file, for a [rollback](#rollback).
4. The checked web app in place, the last block of [The web app](#the-web-app).
5. The release's compose file and Caddy's config, its image in `.env`, and Caddy recreated with them when the stack runs it:

   ```sh self-host=upgrade
   (
     set -eo pipefail
     curl -fsSLO "$SKALD_FILES/docker-compose.yml"
     curl -fsSLO "$SKALD_FILES/Caddyfile"
     sed -i "s|^SKALD_IMAGE=.*|SKALD_IMAGE=ghcr.io/skald-so/skald-server@$SKALD_DIGEST|" .env
     if docker compose ps --services --status running | grep -x caddy >/dev/null; then
       docker compose up --detach --no-deps --force-recreate caddy
     fi
   )
   ```

6. Behind your reverse proxy, with a `compose.proxy.yaml` that turns Caddy off, `profiles: [caddy]`: the stack keeps serving the API, not the web app. For the web app, write the override and your proxy's site block again from [Behind your reverse proxy](#behind-your-reverse-proxy), in your form, and reload your proxy.
7. [Start](#start). The server applies the release's new migrations at start.

### From a release without account recovery

Accounts made on a release without account recovery have no recovery key. An upgrade from such a release needs a new stack, from [Settings](#settings) on, so that each account gets one.

## Rollback

Back to the release before an upgrade, with the backup from that upgrade, `<backup file>`. The rollback replaces the stack's database and attachments with the backup's ([Cloud](dev/infrastructure.md#cloud), "Deployment"): what changed after the upgrade is lost.

In `~/skald`:

1. [The release](#the-release) with the earlier release's values.
2. The backup checked; the stack stopped, its database and attachments volumes removed; the release's files and image in place:

   ```sh self-host=rollback
   (
     set -eo pipefail
     file='<backup file>'
     gzip -t "$file"
     tar -tzf "$file" database.sql >/dev/null
     docker compose down
     docker volume rm skald_database skald_attachments
     curl -fsSLO "$SKALD_FILES/docker-compose.yml"
     curl -fsSLO "$SKALD_FILES/Caddyfile"
     sed -i "s|^SKALD_IMAGE=.*|SKALD_IMAGE=ghcr.io/skald-so/skald-server@$SKALD_DIGEST|" .env
   )
   ```

3. For a release with the web app's archive, `skald-web.tar.gz`: [the web app](#the-web-app), its files, their check and the web app in place.
4. The [restore](#backup-and-restore) of the backup, then [Start](#start).
