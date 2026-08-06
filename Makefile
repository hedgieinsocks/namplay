BIN    := namplay
APP_ID := io.github.hedgieinsocks.Namplay

SCHEMA_DEV := target/schemas
MANIFEST   := $(APP_ID).yaml

.PHONY: all help lint update build run flatpak clean

all: help

help:
	@echo "lint       run cargo clippy"
	@echo "update     update cargo dependencies and regenerate flatpak sources"
	@echo "build      compile debug binary"
	@echo "run        compile debug binary and run with dev schema"
	@echo "flatpak    build distributable flatpak bundle"
	@echo "clean      remove build artifacts"

lint:
	cargo clippy --all-targets -- -D warnings -D clippy::all

update:
	@test -f flatpak-cargo-generator.py || \
		curl -sLO https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
	cargo update
	python3 flatpak-cargo-generator.py Cargo.lock -o cargo-sources.json

build:
	cargo build

run: build
	@mkdir -p $(SCHEMA_DEV)
	glib-compile-schemas data --targetdir=$(SCHEMA_DEV)
	GSETTINGS_SCHEMA_DIR=$(SCHEMA_DEV) RUST_LOG=debug ./target/debug/$(BIN)

flatpak:
	flatpak-builder --repo=repo --force-clean build-dir $(MANIFEST)
	flatpak build-bundle repo $(BIN).flatpak $(APP_ID)

clean:
	cargo clean
	rm -rf $(SCHEMA_DEV) build-dir repo target $(BIN).flatpak
