# ==============================================================================
# RK3588 FaceLiVT & SCRFD 人脸识别算法包构建与打包
# ==============================================================================

# 优先确保 rustup 管理的工具链目录生效
export PATH := $(HOME)/.cargo/bin:$(PATH)

PACKAGE        := face-recognition-rk3588-rknn
TARGET_TRIPLE  := aarch64-unknown-linux-gnu
WORKSPACE_ROOT := ../../../..
ALGO_WORKSPACE_ROOT := $(abspath $(WORKSPACE_ROOT)/algo-packages/rknn/rk3588)
ALGO_MANIFEST := $(ALGO_WORKSPACE_ROOT)/Cargo.toml

.PHONY: help all build host check test run benchmark stress package clean

help: ## 显示此帮助信息
	@echo ""
	@echo "  RK3588 人脸识别算法包"
	@echo ""
	@echo "  构建:"
	@echo "    make                 交叉编译 (Mac -> RK3588)"
	@echo "    make host            本机编译 (在 RK3588 设备上)"
	@echo "    make check           语法检查 (交叉编译目标)"
	@echo ""
	@echo "  测试:"
	@echo "    make test            运行单元测试"
	@echo "    make run             构建 + 本地运行自测"
	@echo "    make benchmark       构建 + 性能基准测试"
	@echo "    make stress          构建 + 压力测试 (默认 100 轮)"
	@echo ""
	@echo "  打包与部署:"
	@echo "    make package         交叉编译 + 打包 (.tar.gz)"
	@echo ""
	@echo "  工具:"
	@echo "    make clean           清理构建产物"
	@echo ""

all: package

# 语法检查
check:
	cd $(WORKSPACE_ROOT) && $(CARGO) check --manifest-path $(ALGO_MANIFEST) -p $(PACKAGE) --all-targets --target $(TARGET_TRIPLE)

# 单元测试
test:
	cd $(WORKSPACE_ROOT) && $(CARGO) test --manifest-path $(ALGO_MANIFEST) -p $(PACKAGE) -- --nocapture

# Cargo 参数
CARGO := cargo

# 检测交叉编译工具链 (优先 cross, 其次 cargo zigbuild, 保底 cargo)
CARGO_CROSS ?= $(shell if command -v cross >/dev/null 2>&1; then echo "CROSS_CONTAINER_ENGINE=podman cross build"; elif cargo zigbuild -h >/dev/null 2>&1; then echo "cargo zigbuild"; else echo "cargo build"; fi)

# 交叉编译 release
build:
	cd $(WORKSPACE_ROOT) && $(CARGO_CROSS) --manifest-path $(ALGO_MANIFEST) -p $(PACKAGE) --release --target $(TARGET_TRIPLE)
	@mkdir -p lib
	cp $(ALGO_WORKSPACE_ROOT)/target/$(TARGET_TRIPLE)/release/libface_recognition_rk3588.so lib/libface_recognition.so

# 目标板原生或本地调试编译 release
host:
	cd $(WORKSPACE_ROOT) && $(CARGO) build --manifest-path $(ALGO_MANIFEST) -p $(PACKAGE) --release
	@mkdir -p lib
	@if [ -f $(ALGO_WORKSPACE_ROOT)/target/release/libface_recognition_rk3588.so ]; then \
		cp $(ALGO_WORKSPACE_ROOT)/target/release/libface_recognition_rk3588.so lib/libface_recognition.so; \
	elif [ -f $(ALGO_WORKSPACE_ROOT)/target/release/libface_recognition_rk3588.dylib ]; then \
		cp $(ALGO_WORKSPACE_ROOT)/target/release/libface_recognition_rk3588.dylib lib/libface_recognition.dylib; \
		echo "提示: 当前在 macOS 本地编译，已生成 lib/libface_recognition.dylib；若需打包 RK3588 Linux 固件制品请使用 'make' 或 'make package'"; \
	else \
		echo "错误: 未找到 release 编译产物"; \
		exit 1; \
	fi

# 打包为标准算法包 (.tar.gz)
package: build
	@if [ ! -f lib/libface_recognition.so ]; then \
		echo "错误: lib/libface_recognition.so 构建未生成"; \
		exit 1; \
	fi
	@HAS_DETECTOR=0; \
	{ [ -f model/scrfd_500m_384x640_rk3588_mixed.rknn ] || [ -f model/scrfd_500m_384x640_mixed.rknn ] || [ -f model/scrfd_2.5g_bnkps_640x384_rk3588_mixed.rknn ]; } && HAS_DETECTOR=1; \
	HAS_EMBEDDER=0; \
	{ [ -f model/facelivtv2_m_realistic_final_super_rk3588_fp16.rknn ] || [ -f model/facelivtv2_l_realistic_final_super_rk3588_fp16.rknn ] || [ -f model/facelivtv2_m_realistic_final_super_fp16.rknn ] || [ -f model/edgeface_base_distill_fp16.rknn ]; } && HAS_EMBEDDER=1; \
	HAS_PERSON=0; \
	{ [ -f model/yolov6n_384x640_rk3588_i8.rknn ] || [ -f model/yolov6n_384x640_i8.rknn ] || [ -f model/yolov8n-640x384-rk3588.rknn ]; } && HAS_PERSON=1; \
	if [ $$HAS_DETECTOR -eq 0 ] || [ $$HAS_EMBEDDER -eq 0 ] || [ $$HAS_PERSON -eq 0 ]; then \
		echo "错误: model/ 下缺少必需的 RKNN 模型权重 (需包含人体检测、人脸检测及人脸特征提取模型)"; \
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

# 本地运行自测
run: build
	@echo "本地运行自测..."
	cd $(WORKSPACE_ROOT) && $(CARGO) run --manifest-path $(ALGO_MANIFEST) -p $(PACKAGE) --release \
		--target $(TARGET_TRIPLE) \
		--bin face_recognition_rk3588_rknn_run_local -- \
		--input algo-packages/rknn/rk3588/face_recognition/testimage.jpg

# 性能基准测试
benchmark: build
	@echo "性能基准测试..."
	cd $(WORKSPACE_ROOT) && $(CARGO) run --manifest-path $(ALGO_MANIFEST) -p $(PACKAGE) --release \
		--target $(TARGET_TRIPLE) \
		--bin face_recognition_rk3588_rknn_run_local -- \
		--input algo-packages/rknn/rk3588/face_recognition/testimage.jpg \
		--loops 20

# 压力测试 (默认 30 秒)
stress: build
	@echo "压力测试..."
	cd $(WORKSPACE_ROOT) && $(CARGO) run --manifest-path $(ALGO_MANIFEST) -p $(PACKAGE) --release \
		--target $(TARGET_TRIPLE) \
		--bin face_recognition_rk3588_rknn_run_local -- \
		--input algo-packages/rknn/rk3588/face_recognition/testimage.jpg \
		--loops 100

clean:
	rm -rf lib face_recognition.tar.gz
