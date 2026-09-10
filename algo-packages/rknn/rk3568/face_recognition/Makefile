# ==============================================================================
# RK3568 EdgeFace 人脸识别算法包构建与打包
# ==============================================================================

PACKAGE        := face-recognition-rk3568-rknn
TARGET_TRIPLE  := aarch64-unknown-linux-gnu
WORKSPACE_ROOT := ../../../..

.PHONY: all build host package clean

all: package

# 交叉编译 release
build:
	cd $(WORKSPACE_ROOT) && cargo build -p $(PACKAGE) --release --target $(TARGET_TRIPLE)
	@mkdir -p lib
	cp $(WORKSPACE_ROOT)/target/$(TARGET_TRIPLE)/release/libface_recognition_rk3568.so lib/libface_recognition.so

# 目标板原生编译 release
host:
	cd $(WORKSPACE_ROOT) && cargo build -p $(PACKAGE) --release
	@mkdir -p lib
	cp $(WORKSPACE_ROOT)/target/release/libface_recognition_rk3568.so lib/libface_recognition.so

# 打包为标准算法包 (.tar.gz)
package:
	@if [ ! -f lib/libface_recognition.so ]; then \
		echo "错误: lib/libface_recognition.so 不存在，请先执行 make build 或 make host"; \
		exit 1; \
	fi
	@if [ ! -f model/yolov8n-face-640x384_rk3568_mixed_face.rknn ] || [ ! -f model/edgeface_xs_gamma_06_rk3568_fp16.rknn ]; then \
		echo "错误: model/ 下缺少必需的 RKNN 模型权重"; \
		exit 1; \
	fi
	rm -f face_recognition.tar.gz
	tar -czf face_recognition.tar.gz \
		--exclude='.DS_Store' \
		--exclude='src' \
		--exclude='Cargo.toml' \
		--exclude='Cargo.lock' \
		--exclude='Makefile' \
		--exclude='*.tar.gz' \
		--exclude='target' \
		--exclude='.env' \
		manifest.json \
		config.schema.json \
		README.md \
		testimage.jpg \
		model \
		lib
	@echo "算法包打包成功: face_recognition.tar.gz"
	@tar -ztvf face_recognition.tar.gz

clean:
	rm -rf lib face_recognition.tar.gz
