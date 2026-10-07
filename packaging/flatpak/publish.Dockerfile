# The tools scripts/publish-flatpak.sh and scripts/flatpak-repo-key.sh run on the Mac, which has neither Flatpak nor
# OSTree: flatpak (build-import-bundle, build-update-repo, build-bundle), ostree and GnuPG, from Debian 13. Nothing runs
# as a Flatpak in here (no bubblewrap or D-Bus needed): these commands only read and write the repository's files.
#
#   docker build -t autopaper-flatpak-publish -f packaging/flatpak/publish.Dockerfile packaging/flatpak
FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends flatpak ostree gnupg ca-certificates \
 && rm -rf /var/lib/apt/lists/*
# GnuPG's homedir is copied in from a read-only mount when signing (see publish-flatpak.sh); nothing is kept here.
ENV GNUPGHOME=/run/gnupg
WORKDIR /work
