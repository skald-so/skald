# The clean machine of script/self-host-test: Debian with Docker from Docker's repository, as a
# self-hoster installs it, run as a privileged container with a dockerd of its own. The tests' own
# tools, which the self-host guide must not need, are off PATH in /opt/self-host-test/bin: Pebble, the
# local ACME server; Caddy, for a reverse proxy on the machine; jq; the GitHub and Google stand-ins,
# which the test copies there. jq joins PATH only for the guide's sign-in checks.
# Its other stages are Caddy's image and the person's browser's, which the test loads into the
# machine's Docker.
FROM ghcr.io/letsencrypt/pebble:2.10.3@sha256:0c81f05db371f436c97fb894bd6d2ab8fc9c8e2ce40f8bef8a8e15941bcd75ef AS pebble
FROM ghcr.io/jqlang/jq:1.8.2@sha256:b9c68867e5766576263a222e91db3de422d802069c7af70440e667a95344e486 AS jq

# Let's Encrypt's stand-in in front of Pebble, and the proxies of the proxy variant.
FROM caddy:2.11.9-alpine@sha256:003a97808e781aa9f7e75cc2f43d34d461099031faf5e20faefb9f61126836f0 AS caddy

# The person's browser: Debian's Chrome Headless Shell and Node.js, for script/lib/self-host/person.mjs.
FROM node:26.13.1-trixie-slim@sha256:abe7e5a82b61a8ef67b315b85497c9e65173387ccf2f4a029248184b8e020be5 AS browser
RUN apt-get update && \
    apt-get install --yes --no-install-recommends chromium-headless-shell && \
    rm -rf /var/lib/apt/lists/*

FROM debian:13.8-slim@sha256:8aa5a7bf5a1955bddbe7d74fcdd0b91ab30e74362a8b73af672b0481b35b44a0 AS machine
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
