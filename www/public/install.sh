#!/bin/sh
# Installs the lane binary. Read it before you run it:
#   curl -fsSL https://lane.lukeed.com
#
#   LANE_VERSION=v0.1.0   pin a release; defaults to the latest
#   LANE_INSTALL=~/bin    where the binary goes; defaults to ~/.local/bin

set -eu

REPO="lukeed/lane"

main() {
	dest=${LANE_INSTALL:-$HOME/.local/bin}
	target=$(detect_target)

	if [ -n "${LANE_VERSION-}" ]; then
		base="https://github.com/$REPO/releases/download/$LANE_VERSION"
		label=$LANE_VERSION
	else
		base="https://github.com/$REPO/releases/latest/download"
		label="the latest release"
	fi

	archive="lane-$target.tar.gz"
	tmp=$(mktemp -d)
	trap 'rm -rf "$tmp"' EXIT INT TERM

	say "downloading lane for $target from $label"
	fetch "$base/$archive" "$tmp/$archive"
	fetch "$base/$archive.sha256" "$tmp/$archive.sha256" 2>/dev/null || true
	verify "$tmp" "$archive"

	tar -xzf "$tmp/$archive" -C "$tmp"
	[ -f "$tmp/lane" ] || die "the archive did not contain a lane binary"

	mkdir -p "$dest"
	install -m 755 "$tmp/lane" "$dest/lane" 2>/dev/null ||
		{ cp "$tmp/lane" "$dest/lane" && chmod 755 "$dest/lane"; }

	say "installed $("$dest/lane" --version) to $dest/lane"

	case ":${PATH}:" in
	*":$dest:"*) ;;
	*)
		say ""
		say "$dest is not on your PATH. Add this to your shell profile:"
		say "  export PATH=\"$dest:\$PATH\""
		;;
	esac

	say ""
	say "next:"
	say "  eval \"\$(lane shellenv)\"      so lane new drops you inside the new lane"
	say "  cd yourproject && lane init   scaffold .lane/, probe reflink"
}

detect_target() {
	os=$(uname -s)
	arch=$(uname -m)

	case $arch in
	arm64 | aarch64) arch=aarch64 ;;
	x86_64 | amd64) arch=x86_64 ;;
	*) die "unsupported architecture: $arch — build from source with cargo install --git https://github.com/$REPO" ;;
	esac

	case $os in
	Darwin) echo "$arch-apple-darwin" ;;
	Linux) echo "$arch-unknown-linux-musl" ;;
	*) die "unsupported system: $os — build from source with cargo install --git https://github.com/$REPO" ;;
	esac
}

fetch() {
	if command -v curl >/dev/null 2>&1; then
		curl -fsSL --proto '=https' --tlsv1.2 -o "$2" "$1"
	elif command -v wget >/dev/null 2>&1; then
		wget -qO "$2" "$1"
	else
		die "no curl and no wget"
	fi
}

# The checksum is served from the same host as the archive, so it catches a
# truncated download rather than a hostile one. Missing tool is not fatal.
verify() {
	[ -f "$1/$2.sha256" ] || { say "no checksum published; skipping verification"; return 0; }

	if command -v sha256sum >/dev/null 2>&1; then
		want=$(cut -d' ' -f1 <"$1/$2.sha256")
		got=$(sha256sum "$1/$2" | cut -d' ' -f1)
	elif command -v shasum >/dev/null 2>&1; then
		want=$(cut -d' ' -f1 <"$1/$2.sha256")
		got=$(shasum -a 256 "$1/$2" | cut -d' ' -f1)
	else
		say "no sha256sum and no shasum; skipping verification"
		return 0
	fi

	[ "$want" = "$got" ] || die "checksum mismatch: expected $want, got $got"
}

say() { printf '%s\n' "$1" >&2; }
die() {
	printf 'install: %s\n' "$1" >&2
	exit 1
}

# Called last so a truncated download cannot half-run.
main "$@"
