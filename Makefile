# Build commands run inside `nix develop` so the GTK toolchain is on PATH.
NIX := nix develop --command
PREFIX := /usr/local
BIN := niri-groom

.PHONY: default debug install

# Release build — the binary I actually run.
default:
	$(NIX) cargo build --release

# Debug build, for quick iteration.
debug:
	$(NIX) cargo build

# Build release and install it system-wide (needs root for $(PREFIX)/bin).
install: default
	sudo install -Dm755 target/release/$(BIN) $(PREFIX)/bin/$(BIN)
