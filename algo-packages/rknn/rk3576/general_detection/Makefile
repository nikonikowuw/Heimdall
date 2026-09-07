.PHONY: all build test run benchmark stress package clean

TARGET_LIB = lib/libgeneral_detection.so
WORKSPACE_ROOT = ../../../..
DURATION ?= 30

all: build

build:
	cargo build -p general-detection-rknn --release
	@mkdir -p lib
	@cp $(WORKSPACE_ROOT)/target/release/libgeneral_detection.so $(TARGET_LIB)

test:
	cargo test -p general-detection-rknn -- --nocapture

run: build
	cargo run -p general-detection-rknn --release --bin run_local

benchmark: build
	cargo run -p general-detection-rknn --release --bin run_local -- --benchmark

stress: build
	cargo run -p general-detection-rknn --release --bin run_local -- --stress $(DURATION)

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
	cargo clean -p general-detection-rknn
	rm -rf lib/*.so general_detection.tar.gz result.jpg
