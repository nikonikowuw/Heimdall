.PHONY: all build test run benchmark stress package clean

TARGET_LIB = lib/libgeneral_detection.dylib
WORKSPACE_ROOT = ../../../..
ALGO_WORKSPACE_ROOT = $(abspath $(WORKSPACE_ROOT)/algo-packages/macos)
ALGO_MANIFEST = $(ALGO_WORKSPACE_ROOT)/Cargo.toml
CARGO = cargo
DURATION ?= 30

all: build

build:
	$(CARGO) build --manifest-path $(ALGO_MANIFEST) -p general-detection --release
	@mkdir -p lib
	@cp $(ALGO_WORKSPACE_ROOT)/target/release/libgeneral_detection_coreml.dylib $(TARGET_LIB)

test:
	$(CARGO) test --manifest-path $(ALGO_MANIFEST) -p general-detection

run: build
	$(CARGO) run --manifest-path $(ALGO_MANIFEST) -p general-detection --release --bin run_local

benchmark: build
	$(CARGO) run --manifest-path $(ALGO_MANIFEST) -p general-detection --release --bin run_local -- --benchmark

stress: build
	$(CARGO) run --manifest-path $(ALGO_MANIFEST) -p general-detection --release --bin run_local -- --stress $(DURATION)

package: build
	@rm -f general_detection.tar.gz
	tar -czf general_detection.tar.gz \
		--exclude=".DS_Store" \
		--exclude="src" \
		--exclude="Cargo.toml" \
		--exclude="Cargo.lock" \
		--exclude="Makefile" \
		--exclude="*.tar.gz" \
		--exclude="result.jpg" \
		--exclude=".env" \
		manifest.json \
		config.schema.json \
		testimage.jpg \
		README.md \
		model \
		lib

clean:
	$(CARGO) clean --manifest-path $(ALGO_MANIFEST) -p general-detection
	rm -rf lib/*.dylib general_detection.tar.gz result.jpg
