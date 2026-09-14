.PHONY: all build test run benchmark stress package clean models models-sync

TARGET_LIB := lib/libface_recognition.dylib
WORKSPACE_ROOT := ../../../..
ALGO_WORKSPACE_ROOT := $(abspath $(WORKSPACE_ROOT)/algo-packages/macos)
ALGO_MANIFEST := $(ALGO_WORKSPACE_ROOT)/Cargo.toml
CARGO := cargo
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
	$(CARGO) build --manifest-path $(ALGO_MANIFEST) -p face-recognition --release
	@mkdir -p lib
	@cp $(ALGO_WORKSPACE_ROOT)/target/release/libface_recognition_coreml.dylib $(TARGET_LIB)

test:
	$(CARGO) test --manifest-path $(ALGO_MANIFEST) -p face-recognition

run: build
	$(CARGO) run --manifest-path $(ALGO_MANIFEST) -p face-recognition --release --bin face_recognition_run_local

benchmark: build
	$(CARGO) run --manifest-path $(ALGO_MANIFEST) -p face-recognition --release --bin face_recognition_run_local -- --benchmark

stress: build
	$(CARGO) run --manifest-path $(ALGO_MANIFEST) -p face-recognition --release --bin face_recognition_run_local -- --stress $(DURATION)

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
	$(CARGO) clean --manifest-path $(ALGO_MANIFEST) -p face-recognition
	rm -rf lib/*.dylib face_recognition.tar.gz result.jpg
