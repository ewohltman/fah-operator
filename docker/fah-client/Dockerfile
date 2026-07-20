# Folding@Home v8 client image bundled by the operator.
# Build context is this directory:
#   docker build -t ghcr.io/ewohltman/fah-client:latest docker/fah-client
#
# The client is configured at runtime from environment variables by
# entrypoint.sh, which renders /fah/config.xml and launches fah-client.

FROM debian:stable-slim

# Latest fah-client at authoring time; override with --build-arg to pin/update.
# Use the debian-10-64bit channel: the debian-stable-64bit channel is frozen at
# 8.1.18, which predates v8 account linking (`account-token`/`machine-name`).
ARG FAH_CLIENT_VERSION=8.5.6
ARG FAH_CLIENT_DEB=https://download.foldingathome.org/releases/public/fah-client/debian-10-64bit/release/fah-client_${FAH_CLIENT_VERSION}_amd64.deb

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates curl; \
    curl -fsSL -o /tmp/fah-client.deb "${FAH_CLIENT_DEB}"; \
    # Pre-create the package's system user/group. The deb's preinst otherwise runs
    # `useradd --groups video,render`, but the `render` group is absent on the slim
    # base image, so that useradd fails, the fah-client user is never created, and
    # the postinst's `install -o fah-client` then aborts dpkg configuration.
    groupadd --system fah-client; \
    useradd --system --gid fah-client --no-create-home \
        --home-dir /var/lib/fah-client --shell /usr/sbin/nologin fah-client; \
    # The .deb postinst also calls `systemctl`, which is absent in a container and
    # would abort dpkg configuration. Shim it with a no-op for the install, then
    # remove the shim.
    printf '#!/bin/sh\nexit 0\n' > /usr/bin/systemctl; \
    chmod +x /usr/bin/systemctl; \
    apt-get install -y --no-install-recommends /tmp/fah-client.deb; \
    rm -f /usr/bin/systemctl; \
    apt-get purge -y --auto-remove curl; \
    rm -f /tmp/fah-client.deb; \
    rm -rf /var/lib/apt/lists/*

# Non-root user with a writable data directory.
RUN useradd --system --uid 10000 --create-home --home-dir /fah fah
WORKDIR /fah

COPY entrypoint.sh /usr/local/bin/entrypoint.sh
# fold.pl starts folding via the client's local WebSocket API (v8 account-linked
# clients start paused); it uses only core Perl, already present in the base image.
COPY fold.pl /usr/local/bin/fold.pl
RUN chmod +x /usr/local/bin/entrypoint.sh /usr/local/bin/fold.pl

USER 10000
# Folding@Home web-control / API port.
EXPOSE 7396
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
