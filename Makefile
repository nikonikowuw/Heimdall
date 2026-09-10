# ============================================================================
#  Heimdall — 统一构建与交叉编译 Makefile
# ============================================================================
#
#  用法:
#    make help              查看全部目标
#    make                   本机构建 (macOS arm64)
#    make rknn              交叉编译 RK3576 主程序
#
#  交叉编译前置条件:
#    1. rustup target add aarch64-unknown-linux-gnu
#    2. cargo install cargo-zigbuild
#    3. brew install zig
#
#  算法包构建请进入对应目录:
#    cd algo-packages/rknn/rk3576/general_detection && make help
#
# ============================================================================

SHELL := /bin/bash
.DEFAULT_GOAL := help

# ──────────────────────────────────────────────────────────────────────────────
# 项目路径
# ──────────────────────────────────────────────────────────────────────────────
WORKSPACE_ROOT := $(shell pwd)
WEB_DIR        := $(WORKSPACE_ROOT)/web
WEB_DIST       := $(WEB_DIR)/dist
ALGO_PKG_DIR   := $(WORKSPACE_ROOT)/algo-packages

# ──────────────────────────────────────────────────────────────────────────────
# 交叉编译目标三元组与部署配置 (可通过环境变量覆盖)
# ──────────────────────────────────────────────────────────────────────────────
RKNN_TARGET       ?= aarch64-unknown-linux-gnu
RKNN_HOST         ?= root@192.168.1.100
RKNN_DEPLOY_PATH  ?= /opt/heimdall
RKNN_SSH_PORT     ?= 22

# ──────────────────────────────────────────────────────────────────────────────
# Cargo 构建参数
# ──────────────────────────────────────────────────────────────────────────────
CARGO       := cargo
RELEASE     := --release

# 主程序交叉编译 feature 组合 (RKNN 平台)
RKNN_APP_FEATURES := media/mpp,infer/backend-rknn

# Rockchip RK SDK sysroot 库搜索路径 (动态交叉编译链接用)
# build.rs 读取 RK_MPP_LIB_DIR 环境变量，此处自动探测 ias-engine-sdk sysroot
RK_MPP_SYSROOT ?= $(HOME)/.cache/ias-engine-sdk/rk3576/sysroot-minimal/usr/lib/aarch64-linux-gnu
RK_MPP_LIB_DIR ?= $(RK_MPP_SYSROOT)

# ──────────────────────────────────────────────────────────────────────────────
# 颜色输出
# ──────────────────────────────────────────────────────────────────────────────
CYAN  := \033[36m
GREEN := \033[32m
YELLOW := \033[33m
RED   := \033[31m
RESET := \033[0m

# ============================================================================
#  帮助
# ============================================================================
.PHONY: help
help: ## 显示此帮助信息
	@echo ""
	@echo "  $(CYAN)Heimdall$(RESET) — 边缘端 AI 视频分析系统构建工具"
	@echo ""
	@echo "  $(GREEN)本机构建 (macOS):$(RESET)"
	@echo "    make build              构建主程序 (debug)"
	@echo "    make build-release      构建主程序 (release)"
	@echo "    make check              语法检查 (主程序)"
	@echo "    make check-all          语法检查 (整个 workspace)"
	@echo "    make test               运行测试"
	@echo "    make clippy             Clippy lint (主程序)"
	@echo "    make clippy-all         Clippy lint (整个 workspace)"
	@echo "    make fmt                代码格式化"
	@echo "    make fmt-check          格式化检查 (CI)"
	@echo ""
	@echo "  $(GREEN)RK3576 交叉编译 ($(RKNN_TARGET)):$(RESET)"
	@echo "    make rknn               交叉编译主程序 (release)"
	@echo "    make rknn-check         交叉编译语法检查"
	@echo "    make rknn-deploy        交叉编译 + SCP 部署到设备"
	@echo ""
	@echo "  $(GREEN)算法包 ($(ALGO_PKG_DIR)/rknn/rk3576/):$(RESET)"
	@echo "    cd algo-packages/rknn/rk3576/general_detection && make"
	@echo "    cd algo-packages/rknn/rk3576/general_detection && make package"
	@echo ""
	@echo "  $(GREEN)前端构建:$(RESET)"
	@echo "    make web                构建前端 SPA"
	@echo "    make web-dev            启动前端开发服务器"
	@echo ""
	@echo "  $(GREEN)工具:$(RESET)"
	@echo "    make setup-cross        检测并安装交叉编译依赖 (一键)"
	@echo "    make verify-cross       验证交叉编译工具链是否就绪"
	@echo "    make clean              清理所有构建产物"
	@echo "    make clean-rknn         仅清理 RKNN 交叉编译产物"
	@echo ""
	@echo "  $(YELLOW)部署配置 (环境变量):$(RESET)"
	@echo "    RKNN_HOST=root@192.168.1.100    设备 SSH 地址"
	@echo "    RKNN_DEPLOY_PATH=/opt/heimdall  设备部署路径"
	@echo "    RKNN_SSH_PORT=22                SSH 端口"
	@echo ""

# ============================================================================
#  前置校验
# ============================================================================

.PHONY: setup-cross
setup-cross: ## 检测并安装交叉编译依赖
	@$(WORKSPACE_ROOT)/scripts/setup-cross-deps.sh

.PHONY: verify-cross
verify-cross: ## 验证交叉编译工具链
	@$(WORKSPACE_ROOT)/scripts/setup-cross-deps.sh --check

# 确保 web/dist 存在且最新 (rust-embed 需要)
WEB_SRCS := $(shell find $(WEB_DIR)/src $(WEB_DIR)/public -type f 2>/dev/null) $(WEB_DIR)/package.json $(WEB_DIR)/index.html

$(WEB_DIST)/index.html: $(WEB_SRCS)
	@echo -e "$(CYAN)[web]$(RESET) 前端源码有变更，自动触发前端构建..."
	cd $(WEB_DIR) && pnpm build

.PHONY: ensure-web-dist
ensure-web-dist: $(WEB_DIST)/index.html

# ============================================================================
#  本机构建 (macOS arm64)
# ============================================================================

.PHONY: build
build: ensure-web-dist ## 构建主程序 (debug, macOS)
	$(CARGO) build -p app

.PHONY: build-release
build-release: ensure-web-dist ## 构建主程序 (release, macOS)
	$(CARGO) build -p app $(RELEASE)

.PHONY: check
check: ## 语法与类型检查 (主程序)
	$(CARGO) check -p app

.PHONY: check-all
check-all: ## 语法检查 (整个 workspace)
	$(CARGO) check --workspace

.PHONY: test
test: ## 运行测试 (主程序相关)
	$(CARGO) test -p app -p types -p api -p infer

.PHONY: clippy
clippy: ## Clippy lint 检查 (主程序)
	$(CARGO) clippy -p app -- -D warnings

.PHONY: clippy-all
clippy-all: ## Clippy lint 检查 (整个 workspace)
	$(CARGO) clippy --workspace -- -D warnings

.PHONY: fmt
fmt: ## 代码格式化
	$(CARGO) fmt --all

.PHONY: fmt-check
fmt-check: ## 格式化检查 (CI 用)
	$(CARGO) fmt --all -- --check

# ============================================================================
#  RK3576 交叉编译 — 主程序
# ============================================================================

.PHONY: rknn
rknn: ensure-web-dist verify-cross ## [交叉编译] 主程序 for RK3576 (release)
	@echo -e "$(CYAN)[rknn]$(RESET) 交叉编译主程序 → $(RKNN_TARGET)..."
	RK_MPP_LIB_DIR=$(RK_MPP_LIB_DIR) \
	$(CARGO) zigbuild -p app $(RELEASE) \
		--target $(RKNN_TARGET) \
		--features $(RKNN_APP_FEATURES)
	@echo -e "$(GREEN)[rknn]$(RESET) 产物: target/$(RKNN_TARGET)/release/heimdall"

.PHONY: rknn-check
rknn-check: verify-cross ## [交叉编译] 主程序语法检查
	$(CARGO) check -p app \
		--target $(RKNN_TARGET) \
		--features $(RKNN_APP_FEATURES)

.PHONY: rknn-deploy
rknn-deploy: rknn ## [交叉编译] 主程序部署到 RK3576 设备
	@echo -e "$(CYAN)[rknn-deploy]$(RESET) 部署到 $(RKNN_HOST):$(RKNN_DEPLOY_PATH)..."
	@ssh -p $(RKNN_SSH_PORT) $(RKNN_HOST) "mkdir -p $(RKNN_DEPLOY_PATH)/bin"
	@scp -P $(RKNN_SSH_PORT) \
		$(WORKSPACE_ROOT)/target/$(RKNN_TARGET)/release/heimdall \
		$(RKNN_HOST):$(RKNN_DEPLOY_PATH)/bin/
	@echo -e "$(GREEN)[rknn-deploy]$(RESET) 部署完成: $(RKNN_HOST):$(RKNN_DEPLOY_PATH)/bin/heimdall"

# ============================================================================
#  前端构建
# ============================================================================

.PHONY: web
web: ## 构建前端 SPA
	@echo -e "$(CYAN)[web]$(RESET) 构建前端..."
	cd $(WEB_DIR) && pnpm build
	@echo -e "$(GREEN)[web]$(RESET) 前端产物: $(WEB_DIST)"

.PHONY: web-dev
web-dev: ## 启动前端开发服务器
	cd $(WEB_DIR) && pnpm dev

# ============================================================================
#  清理
# ============================================================================

.PHONY: clean
clean: clean-rknn ## 清理所有构建产物
	$(CARGO) clean
	@echo -e "$(GREEN)[clean]$(RESET) 已清理全部构建产物"

.PHONY: clean-rknn
clean-rknn: ## 仅清理 RKNN 交叉编译产物
	@rm -rf $(WORKSPACE_ROOT)/target/$(RKNN_TARGET)
	@echo -e "$(GREEN)[clean-rknn]$(RESET) 已清理 RKNN 交叉编译产物"

# ============================================================================
#  开发辅助
# ============================================================================

.PHONY: stats
stats: ## 查看项目代码统计
	@echo -e "$(CYAN)[stats]$(RESET) Rust 代码行数:"
	@find $(WORKSPACE_ROOT)/crates -name "*.rs" ! -path "*/target/*" | xargs wc -l | tail -1
	@echo -e "$(CYAN)[stats]$(RESET) 算法包代码行数:"
	@find $(ALGO_PKG_DIR) -name "*.rs" ! -path "*/target/*" | xargs wc -l 2>/dev/null | tail -1

.PHONY: tree-algo
tree-algo: ## 查看算法包目录结构
	@echo -e "$(CYAN)RK3576 算法包结构:$(RESET)"
	@find $(ALGO_PKG_DIR)/rknn/rk3576/general_detection -maxdepth 2 \
		! -path "*/target/*" ! -path "*/.git/*" | sort | sed 's|$(ALGO_PKG_DIR)/rknn/rk3576/||'
