BIN    := namplay
APP_ID := io.github.hedgieinsocks.Namplay
PREFIX ?= $(HOME)/.local

ICON_SRC    := data/$(APP_ID).png
ICON_DST    := $(PREFIX)/share/icons/hicolor/128x128/apps/$(APP_ID).png
DESKTOP_SRC := data/$(APP_ID).desktop
DESKTOP_DST := $(PREFIX)/share/applications/$(APP_ID).desktop
SCHEMA_SRC  := data/$(APP_ID).gschema.xml
SCHEMA_DST  := $(PREFIX)/share/glib-2.0/schemas/$(APP_ID).gschema.xml
SCHEMA_DEV  := target/schemas
MANIFEST    := $(APP_ID).yaml

.PHONY: all help build run release install uninstall flatpak clean

all: help

help:
	@echo "build      compile debug binary"
	@echo "run        compile and run with dev schema"
	@echo "release    compile optimized binary"
	@echo "install    install to $(PREFIX)"
	@echo "uninstall  remove installed files"
	@echo "flatpak    build distributable flatpak bundle"
	@echo "clean      remove build artifacts"

build:
	cargo build

run: build
	@mkdir -p $(SCHEMA_DEV)
	glib-compile-schemas data --targetdir=$(SCHEMA_DEV)
	GSETTINGS_SCHEMA_DIR=$(SCHEMA_DEV) ./target/debug/$(BIN)

release:
	cargo build --release

install: release
	install -Dm755 target/release/$(BIN)  $(PREFIX)/bin/$(BIN)
	install -Dm644 $(ICON_SRC)            $(ICON_DST)
	sed "s|Exec=$(BIN)|Exec=$(PREFIX)/bin/$(BIN)|" $(DESKTOP_SRC) | install -Dm644 /dev/stdin $(DESKTOP_DST)
	install -Dm644 $(SCHEMA_SRC)          $(SCHEMA_DST)
	glib-compile-schemas $(PREFIX)/share/glib-2.0/schemas
	gtk-update-icon-cache -qtf $(PREFIX)/share/icons/hicolor
	update-desktop-database $(PREFIX)/share/applications

uninstall:
	rm -f $(PREFIX)/bin/$(BIN)
	rm -f $(ICON_DST)
	rm -f $(DESKTOP_DST)
	rm -f $(SCHEMA_DST)
	glib-compile-schemas $(PREFIX)/share/glib-2.0/schemas
	gtk-update-icon-cache -qtf $(PREFIX)/share/icons/hicolor
	update-desktop-database $(PREFIX)/share/applications

flatpak:
	flatpak-builder --repo=repo --force-clean build-dir $(MANIFEST)
	flatpak build-bundle repo $(BIN).flatpak $(APP_ID)

clean:
	cargo clean
	rm -rf $(SCHEMA_DEV) build-dir repo $(BIN).flatpak
