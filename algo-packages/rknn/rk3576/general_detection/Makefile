# ============================================================================
#  RK3576 通用目标检测算法包 — 构建与打包
# ============================================================================
#
#  用法:
#    make                交叉编译 (需配置交叉工具链)
#    make host           本机编译 (仅在 RK3576 设备上或 Linux 开发机)
#    make test           运行单元测试
#    make package        交叉编译 + 打包 (.tar.gz)
#    make deploy         交叉编译 + 打包 + SCP 部署
#    make clean          清理构建产物
#
#  交叉编译前置条件 (在 macOS 上执行):
#    1. rustup target add aarch64-unknown-linux-gnu
#    2. cargo install cargo-zigbuild && brew install zig
#
# ============================================================================

SHELL := /bin/bash
.DEFAULT_GOAL := all

# ──────────────────────────────────────────────────────────────────────────────
# 路径
# ──────────────────────────────────────────────────────────────────────────────
WORKSPACE_ROOT  := ../../../..
TARGET_LIB      := lib/libgeneral_detection.so
TARGET_LIB_HOST := lib/libgeneral_detection.so

# ──────────────────────────────────────────────────────────────────────────────
# 交叉编译配置 (可通过环境变量覆盖)
# ──────────────────────────────────────────────────────────────────────────────
CROSS_TARGET    ?= aarch64-unknown-linux-gnu
RKNN_HOST       ?= root@192.168.1.100
RKNN_DEPLOY_PATH ?= /opt/heimdall/algo-packages
RKNN_SSH_PORT   ?= 22

# ──────────────────────────────────────────────────────────────────────────────
# Cargo 参数
# ──────────────────────────────────────────────────────────────────────────────
CARGO           := cargo
CROSS_FEATURES  ?= rga,testing-hardware
DURATION        ?= 30

# ──────────────────────────────────────────────────────────────────────────────
# 颜色
# ──────────────────────────────────────────────────────────────────────────────
CYAN  := \033[36m
GREEN := \033[32m
RESET := \033[0m

# ============================================================================
#  帮助
# ============================================================================

.PHONY: help
help: ## 显示此帮助信息
	@echo ""
	@echo "  $(CYAN)RK3576 通用目标检测算法包$(RESET)"
	@echo ""
	@echo "  $(GREEN)构建:$(RESET)"
	@echo "    make                 交叉编译 (Mac → RK3576)"
	@echo "    make host            本机编译 (在 RK3576 设备上)"
	@echo "    make check           语法检查 (交叉编译目标)"
	@echo ""
	@echo "  $(GREEN)测试:$(RESET)"
	@echo "    make test            运行单元测试"
	@echo "    make run             构建 + 本地运行自测"
	@echo "    make benchmark       构建 + 性能基准测试"
	@echo "    make stress          构建 + 压力测试 (DURATION=30s)"
	@echo ""
	@echo "  $(GREEN)打包与部署:$(RESET)"
	@echo "    make package         交叉编译 + 打包 (.tar.gz)"
	@echo "    make deploy          交叉编译 + 打包 + SCP 部署"
	@echo ""
	@echo "  $(GREEN)工具:$(RESET)"
	@echo "    make clean           清理构建产物"
	@echo ""
	@echo "  $(YELLOW)配置 (环境变量):$(RESET)"
	@echo "    CROSS_TARGET=aarch64-unknown-linux-gnu"
	@echo "    RKNN_HOST=root@192.168.1.100"
	@echo ""

# ============================================================================
#  默认: 交叉编译
# ============================================================================

.PHONY: all
all: build

# ============================================================================
#  交叉编译 (Mac → RK3576)
# ============================================================================

.PHONY: build
build: ## 交叉编译算法包 (release, → aarch64-linux)
	@echo -e "$(CYAN)[build]$(RESET) 交叉编译 → $(CROSS_TARGET)..."
	cd $(WORKSPACE_ROOT) && $(CARGO) zigbuild -p general-detection-rknn --release \
		--target $(CROSS_TARGET) \
		--features $(CROSS_FEATURES)
	@mkdir -p lib
	@cp $(WORKSPACE_ROOT)/target/$(CROSS_TARGET)/release/libgeneral_detection.so \
		$(TARGET_LIB)
	@echo -e "$(GREEN)[build]$(RESET) 产物: $(TARGET_LIB)"

# ============================================================================
#  本机编译 (在 RK3576 设备上或 Linux 开发机上)
# ============================================================================

.PHONY: host
host: ## 本机编译算法包 (release, 仅 Linux 设备上使用)
	@echo -e "$(CYAN)[host]$(RESET) 本机编译..."
	cd $(WORKSPACE_ROOT) && $(CARGO) build -p general-detection-rknn --release
	@mkdir -p lib
	@cp $(WORKSPACE_ROOT)/target/release/libgeneral_detection.so \
		$(TARGET_LIB_HOST)
	@echo -e "$(GREEN)[host]$(RESET) 产物: $(TARGET_LIB_HOST)"

# ============================================================================
#  测试
# ============================================================================

.PHONY: test
test: ## 运行单元测试
	cd $(WORKSPACE_ROOT) && $(CARGO) test -p general-detection-rknn -- --nocapture

.PHONY: check
check: ## 语法与类型检查 (交叉编译目标)
	cd $(WORKSPACE_ROOT) && $(CARGO) check -p general-detection-rknn \
		--target $(CROSS_TARGET) \
		--features $(CROSS_FEATURES)

# ============================================================================
#  打包
# ============================================================================

.PHONY: package
package: build ## 交叉编译 + 打包算法包 (.tar.gz)
	@echo -e "$(CYAN)[package]$(RESET) 打包算法包..."
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
	@echo -e "$(GREEN)[package]$(RESET) 包产物: general_detection.tar.gz"

# ============================================================================
#  部署
# ============================================================================

.PHONY: deploy
deploy: package ## 交叉编译 + 打包 + SCP 部署到 RK3576 设备
	@echo -e "$(CYAN)[deploy]$(RESET) 部署到 $(RKNN_HOST):$(RKNN_DEPLOY_PATH)..."
	@ssh -p $(RKNN_SSH_PORT) $(RKNN_HOST) "mkdir -p $(RKNN_DEPLOY_PATH)/general_detection"
	@scp -P $(RKNN_SSH_PORT) -r \
		manifest.json \
		config.schema.json \
		testimage.jpg \
		README.md \
		model \
		lib \
		$(RKNN_HOST):$(RKNN_DEPLOY_PATH)/general_detection/
	@echo -e "$(GREEN)[deploy]$(RESET) 部署完成"

# ============================================================================
#  本地运行 (在设备上或带 librknnrt.so 的开发机)
# ============================================================================

.PHONY: run
run: build ## 构建 + 本地运行自测
	@echo -e "$(CYAN)[run]$(RESET) 本地运行自测..."
	cd $(WORKSPACE_ROOT) && $(CARGO) run -p general-detection-rknn --release \
		--target $(CROSS_TARGET) \
		--features $(CROSS_FEATURES) \
		--bin run_local

.PHONY: benchmark
benchmark: build ## 构建 + 性能基准测试
	cd $(WORKSPACE_ROOT) && $(CARGO) run -p general-detection-rknn --release \
		--target $(CROSS_TARGET) \
		--features $(CROSS_FEATURES) \
		--bin run_local -- --benchmark

.PHONY: stress
stress: build ## 构建 + 压力测试 (默认 30 秒)
	cd $(WORKSPACE_ROOT) && $(CARGO) run -p general-detection-rknn --release \
		--target $(CROSS_TARGET) \
		--features $(CROSS_FEATURES) \
		--bin run_local -- --stress $(DURATION)

# ============================================================================
#  清理
# ============================================================================

.PHONY: clean
clean: ## 清理构建产物
	cd $(WORKSPACE_ROOT) && $(CARGO) clean -p general-detection-rknn
	rm -rf lib/*.so lib/*.dylib general_detection.tar.gz result.jpg
	@echo -e "$(GREEN)[clean]$(RESET) 已清理"
