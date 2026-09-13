.PHONY: all build test run benchmark stress package clean models models-sync

TARGET_LIB := lib/libface_recognition.dylib
WORKSPACE_ROOT := ../../../..
DURATION ?= 30
REQUIRED_MODELS := model/edgeface_s.mlpackage model/yolo26n.mlpackage model/yolov8_face.mlpackage

all: build

models:
	python3 convert_models.py

# 从 general_detection 同步 yolo26n 人体检测模型
models-sync:
	@rm -rf model/yolo26n.mlpackage
	@cp -R ../general_detection/model/yolo26n.mlpackage model/yolo26n.mlpackage
	@echo "Synced yolo26n.mlpackage from general_detection"

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
	@echo "Checking mlpackage weights..."
	@for pkg in $(REQUIRED_MODELS); do \
		if [ ! -d "$$pkg/Data/com.apple.CoreML/weights" ]; then \
			echo "ERROR: $$pkg is missing Data/com.apple.CoreML/weights/"; \
			echo "Run 'make models' first to generate CoreML model weights."; \
			exit 1; \
		fi; \
	done
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
		model/README.md \
		$(REQUIRED_MODELS) \
		lib

clean:
	cargo clean -p face-recognition
	rm -rf lib/*.dylib face_recognition.tar.gz result.jpg
