.PHONY: all build test run benchmark stress package clean models

TARGET_LIB := lib/libface_recognition.dylib
WORKSPACE_ROOT := ../../../..
DURATION ?= 30

all: build

models:
	python3 convert_models.py

build:
	cargo build -p face-recognition --release
	@mkdir -p lib
	@cp $(WORKSPACE_ROOT)/target/release/libface_recognition_coreml.dylib $(TARGET_LIB)

test:
	cargo test -p face-recognition

run: build
	cargo run -p face-recognition --release --bin face_recognition_run_local

benchmark: build
	cargo run -p face-recognition --release --bin face_recognition_run_local -- --benchmark

stress: build
	cargo run -p face-recognition --release --bin face_recognition_run_local -- --stress $(DURATION)

package: build
	@rm -f face_recognition.tar.gz
	tar -czf face_recognition.tar.gz \
		--exclude=".DS_Store" \
		--exclude="src" \
		--exclude="Cargo.toml" \
		--exclude="Cargo.lock" \
		--exclude="Makefile" \
		--exclude="*.tar.gz" \
		--exclude="result.jpg" \
		--exclude=".env" \
		--exclude="convert_models.py" \
		manifest.json \
		config.schema.json \
		testimage.jpg \
		README.md \
		model \
		lib

clean:
	cargo clean -p face-recognition
	rm -rf lib/*.dylib face_recognition.tar.gz result.jpg
