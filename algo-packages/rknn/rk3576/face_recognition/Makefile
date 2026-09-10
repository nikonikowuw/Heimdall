SHELL := /bin/bash
.DEFAULT_GOAL := help

WORKSPACE_ROOT := ../../../..
PACKAGE := face-recognition-rknn
TARGET ?= aarch64-unknown-linux-gnu
CARGO ?= cargo
CARGO_ZIGBUILD ?= cargo zigbuild
LIB := lib/libface_recognition.so

.PHONY: help build host check test hardware package clean

help:
	@printf '%s\n' \
		'make build     cross-compile the RK3576 release library' \
		'make host      build a native release library' \
		'make check     cross-check Rust targets and all package targets' \
		'make test      run host unit tests' \
		'make hardware  run ignored tests on a RK3576 board' \
		'make package   build and create face_recognition.tar.gz' \
		'make clean     remove package build artifacts'

build:
	cd $(WORKSPACE_ROOT) && $(CARGO_ZIGBUILD) build -p $(PACKAGE) --release --target $(TARGET)
	@mkdir -p lib
	cp $(WORKSPACE_ROOT)/target/$(TARGET)/release/libface_recognition.so $(LIB)

host:
	cd $(WORKSPACE_ROOT) && $(CARGO) build -p $(PACKAGE) --release
	@mkdir -p lib
	cp $(WORKSPACE_ROOT)/target/release/libface_recognition.so $(LIB)

check:
	cd $(WORKSPACE_ROOT) && $(CARGO) check -p $(PACKAGE) --all-targets --target $(TARGET)

test:
	cd $(WORKSPACE_ROOT) && $(CARGO) test -p $(PACKAGE) --lib

hardware:
	cd $(WORKSPACE_ROOT) && $(CARGO) test -p $(PACKAGE) -- --ignored --nocapture

package: build
	@rm -f face_recognition.tar.gz
	tar -czf face_recognition.tar.gz \
		--exclude='.DS_Store' \
		--exclude='src' \
		--exclude='Cargo.toml' \
		--exclude='Cargo.lock' \
		--exclude='Makefile' \
		--exclude='*.tar.gz' \
		--exclude='target' \
		--exclude='.env' \
		manifest.json config.schema.json README.md testimage.jpg model lib

clean:
	rm -rf lib face_recognition.tar.gz
