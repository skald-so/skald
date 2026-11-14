# The clean machine of script/self-host-test: Debian with Docker from Docker's repository, as a
# self-hoster installs it, run as a privileged container with a dockerd of its own. The tests' own
# tools, which the self-host guide must not need, are off PATH in /opt/self-host-test/bin: Pebble, the
# local ACME server; Caddy, for a reverse proxy on the machine; jq; the GitHub and Google stand-ins,
# which the test copies there. jq joins PATH only for the guide's sign-in checks.
# Its other stages are the stand-ins' images and the person's browser's, which the test loads into the
# machine's Docker.
FROM ghcr.io/letsencrypt/pebble:2.10.3@sha256:0c81f05db371f436c97fb894bd6d2ab8fc9c8e2ce40f8bef8a8e15941bcd75ef AS pebble
FROM ghcr.io/jqlang/jq:1.8.1@sha256:4f34c6d23f4b1372ac789752cc955dc67c2ae177eb1b5860b75cdc5091ce6f91 AS jq

# Let's Encrypt's stand-in in front of Pebble, and the proxies of the proxy variant.
FROM caddy:2.11.9-alpine@sha256:003a97808e781aa9f7e75cc2f43d34d461099031faf5e20faefb9f61126836f0 AS caddy

# The person's browser: Debian's Chrome Headless Shell and Node.js, for script/lib/self-host/person.mjs.
FROM node:26.13.0-trixie-slim@sha256:5adad64f9a1fca2d51972caa65983634c8a774eb300d72b20f7f2a0f60368072 AS browser
RUN apt-get update && \
    apt-get install --yes --no-install-recommends chromium-headless-shell && \
    rm -rf /var/lib/apt/lists/*

# The bucket's stand-in: an S3-compatible server.
FROM versity/versitygw:v1.11.0@sha256:8319a530d08d537426e2f855bae5280709795dc73a3f0c347973ef915d6343c5 AS bucket

FROM debian:13.4-slim@sha256:109e2c65005bf160609e4ba6acf7783752f8502ad218e298253428690b9eaa4b AS machine
RUN apt-get update && \
    apt-get install --yes --no-install-recommends ca-certificates curl gnupg iptables procps && \
    install -m 0755 -d /etc/apt/keyrings && \
    curl -fsSL https://download.docker.com/linux/debian/gpg -o /etc/apt/keyrings/docker.asc && \
    echo "deb [signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/debian trixie stable" \
      >/etc/apt/sources.list.d/docker.list && \
    apt-get update && \
    apt-get install --yes --no-install-recommends docker-ce docker-ce-cli containerd.io \
      docker-buildx-plugin docker-compose-plugin && \
    rm -rf /var/lib/apt/lists/*
COPY --from=pebble /app /opt/self-host-test/bin/pebble
COPY --from=jq /jq /opt/self-host-test/bin/jq
# What the sign-in checks need besides curl, apart from the other tools.
RUN mkdir -p /opt/self-host-test/checks && ln -s /opt/self-host-test/bin/jq /opt/self-host-test/checks/jq
COPY --from=caddy /usr/bin/caddy /opt/self-host-test/bin/caddy
# dockerd in a container on cgroup v2: the container's processes move to a child group, so the
# root group can delegate its controllers, as the docker:dind image does.
COPY <<'EOF' /opt/self-host-test/bin/dockerd-start
#!/bin/sh
set -e
if [ -f /sys/fs/cgroup/cgroup.controllers ] && [ ! -d /sys/fs/cgroup/init ]; then
  mkdir -p /sys/fs/cgroup/init
  xargs -rn1 </sys/fs/cgroup/cgroup.procs >/sys/fs/cgroup/init/cgroup.procs 2>/dev/null || :
  sed -e 's/ / +/g' -e 's/^/+/' </sys/fs/cgroup/cgroup.controllers >/sys/fs/cgroup/cgroup.subtree_control
fi
exec dockerd >>/var/log/dockerd.log 2>&1
EOF
RUN chmod +x /opt/self-host-test/bin/dockerd-start
VOLUME /var/lib/docker
CMD ["sleep", "infinity"]
