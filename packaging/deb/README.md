# Debian package metadata notes (Phase D / #235)
#
# Prefer `./scripts/package-deb.sh`, which builds:
#   dist/quon_${version}_${amd64|arm64}.deb
# packing target/release/{quonc,quonfmt,quon_lsp,quonlint} into /usr/bin.
#
# The release workflow runs package-deb after the static link audit on Linux
# and attaches the .deb to the GitHub Release. Users install with:
#
#   sudo apt install ./quon_*.deb
#
# An APT repository / Packagecloud mirror is intentionally out of scope.
