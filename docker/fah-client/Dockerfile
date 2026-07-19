# Folding@Home v8 client image bundled by the operator.
# Build context is this directory:
#   docker build -t ghcr.io/ewohltman/fah-client:latest docker/fah-client
#
# The client is configured at runtime from environment variables by
# entrypoint.sh, which renders /fah/config.xml and launches fah-client.

FROM debian:stable-slim

# Latest fah-client at authoring time; override with --build-arg to pin/update.
ARG FAH_CLIENT_VERSION=8.1.18
ARG FAH_CLIENT_DEB=https://download.foldingathome.org/releases/public/fah-client/debian-stable-64bit/release/fah-client_${FAH_CLIENT_VERSION}_amd64.deb

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates curl; \
    # Prevent the .deb postinst from trying to start a systemd service.
    printf '#!/bin/sh\nexit 101\n' > /usr/sbin/policy-rc.d; \
    chmod +x /usr/sbin/policy-rc.d; \
    curl -fsSL -o /tmp/fah-client.deb "${FAH_CLIENT_DEB}"; \
    apt-get install -y --no-install-recommends /tmp/fah-client.deb; \
    apt-get purge -y --auto-remove curl; \
    rm -f /tmp/fah-client.deb /usr/sbin/policy-rc.d; \
    rm -rf /var/lib/apt/lists/*

# Non-root user with a writable data directory.
RUN useradd --system --uid 10000 --create-home --home-dir /fah fah
WORKDIR /fah

COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

USER 10000
# Folding@Home web-control / API port.
EXPOSE 7396
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
