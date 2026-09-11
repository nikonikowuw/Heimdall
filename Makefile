# ============================================================================
#  Heimdall — 统一构建与交叉编译 Makefile
# ============================================================================
#
#  用法:
#    make help              查看全部目标
#    make                   本机构建 (macOS arm64)
#    make sdk-sync          从设备同步 SDK 库
#    make cross             交叉编译 RK3576 主程序
#
#  交叉编译前置条件:
#    1. rustup target add aarch64-unknown-linux-gnu
#    2. cargo install cargo-zigbuild
#    3. brew install zig
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
SDK_LIBS_DIR   := $(WORKSPACE_ROOT)/.rk-sdk-libs

# ──────────────────────────────────────────────────────────────────────────────
# 交叉编译配置 (可通过环境变量覆盖)
# ──────────────────────────────────────────────────────────────────────────────
RKNN_TARGET       ?= aarch64-unknown-linux-gnu
RKNN_DEVICE       ?= rk3576
RKNN_HOST         ?= root@192.168.1.100
RKNN_DEPLOY_PATH  ?= /opt/heimdall
RKNN_SSH_PORT     ?= 22

# SDK 库路径：优先使用环境变量，否则从 .rk-sdk-libs/<device>/ 读取
RK_MPP_LIB_DIR    ?= $(SDK_LIBS_DIR)/$(RKNN_DEVICE)

# ──────────────────────────────────────────────────────────────────────────────
# Cargo 构建参数
# ──────────────────────────────────────────────────────────────────────────────
CARGO       := cargo
RELEASE     := --release

# 主程序交叉编译 feature 组合 (RKNN 平台)
RKNN_APP_FEATURES := media/mpp,infer/backend-rknn

# ──────────────────────────────────────────────────────────────────────────────
# 颜色输出
# ──────────────────────────────────────────────────────────────────────────────
CYAN    := \033[36m
GREEN   := \033[32m
YELLOW  := \033[33m
RED     := \033[31m
RESET   := \033[0m

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
	@echo "  $(GREEN)SDK 库管理:$(RESET)"
	@echo "    make sdk-sync           从设备同步 Rockchip SDK 库"
	@echo "    make sdk-sync RKNN_HOST=root@192.168.1.100   指定设备 IP"
	@echo "    make sdk-sync RKNN_DEVICE=rk3568             指定设备型号"
	@echo ""
	@echo "  $(GREEN)交叉编译 ($(RKNN_TARGET)):$(RESET)"
	@echo "    make cross              交叉编译主程序 (默认 RK3576)"
	@echo "    make cross-rk3576       交叉编译 RK3576 版本"
	@echo "    make cross-rk3568       交叉编译 RK3568 版本"
	@echo "    make cross-check        交叉编译语法检查"
	@echo ""
	@echo "  $(GREEN)部署:$(RESET)"
	@echo "    make deploy             部署到设备 (默认 RK3576)"
	@echo "    make deploy-rk3576      部署 RK3576 版本"
	@echo "    make deploy-rk3568      部署 RK3568 版本"
	@echo ""
	@echo "  $(GREEN)前端:$(RESET)"
	@echo "    make web                构建前端 SPA"
	@echo "    make web-dev            启动前端开发服务器"
	@echo ""
	@echo "  $(GREEN)工具:$(RESET)"
	@echo "    make setup-cross        检测并安装交叉编译依赖"
	@echo "    make verify-cross       验证交叉编译工具链"
	@echo "    make clean              清理所有构建产物"
	@echo ""
	@echo "  $(YELLOW)配置 (环境变量):$(RESET)"
	@echo "    RKNN_HOST=root@192.168.1.100    设备 SSH 地址"
	@echo "    RKNN_DEVICE=rk3576              目标设备型号"
	@echo "    RKNN_DEPLOY_PATH=/opt/heimdall  部署路径"
	@echo "    RK_MPP_LIB_DIR=/path/to/libs    手动指定 SDK 库路径"
	@echo ""

# ============================================================================
#  前置校验与依赖
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
#  SDK 库管理
# ============================================================================

.PHONY: sdk-sync
sdk-sync: ## 从设备同步 Rockchip SDK 库到 .rk-sdk-libs/
	@$(WORKSPACE_ROOT)/scripts/sdk-sync.sh \
		--host "$(RKNN_HOST)" \
		--device "$(RKNN_DEVICE)" \
		--port "$(RKNN_SSH_PORT)"

.PHONY: sdk-check
sdk-check: ## 检查 SDK 库是否就绪
	@echo -e "$(CYAN)[sdk-check]$(RESET) 检查设备 $(RKNN_DEVICE) SDK 库..."
	@if [ ! -d "$(RK_MPP_LIB_DIR)" ]; then \
		echo -e "$(RED)[FAIL]$(RESET) 未找到 SDK 库目录: $(RK_MPP_LIB_DIR)"; \
		echo -e "$(YELLOW)[HINT]$(RESET) 请执行: make sdk-sync"; \
		exit 1; \
	fi
	@for lib in librockchip_mpp.so librga.so librknnrt.so; do \
		if [ -f "$(RK_MPP_LIB_DIR)/$$lib" ]; then \
			echo -e "$(GREEN)[  OK]$(RESET) $$lib"; \
		else \
			echo -e "$(YELLOW)[WARN]$(RESET) $$lib — 未找到 (可选)"; \
		fi; \
	done

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
#  交叉编译 — 主程序
# ============================================================================

# 内部目标：执行交叉编译
.PHONY: _cross
_cross: ensure-web-dist verify-cross sdk-check
	@echo -e "$(CYAN)[cross]$(RESET) 交叉编译主程序 → $(RKNN_TARGET) ($(RKNN_DEVICE))..."
	@echo -e "$(CYAN)[cross]$(RESET) SDK 库路径: $(RK_MPP_LIB_DIR)"
	RK_MPP_LIB_DIR=$(RK_MPP_LIB_DIR) \
	$(CARGO) zigbuild -p app $(RELEASE) \
		--target $(RKNN_TARGET) \
		--features $(RKNN_APP_FEATURES)
	@echo -e "$(GREEN)[cross]$(RESET) 产物: target/$(RKNN_TARGET)/release/heimdall"

# 用户命令
.PHONY: cross
cross: _cross ## 交叉编译主程序 (默认 RK3576)

.PHONY: cross-rk3576
cross-rk3576: ## 交叉编译 RK3576 版本
	$(MAKE) _cross RKNN_DEVICE=rk3576

.PHONY: cross-rk3568
cross-rk3568: ## 交叉编译 RK3568 版本
	$(MAKE) _cross RKNN_DEVICE=rk3568

.PHONY: cross-check
cross-check: verify-cross sdk-check ## 交叉编译语法检查
	$(CARGO) check -p app \
		--target $(RKNN_TARGET) \
		--features $(RKNN_APP_FEATURES)

# ============================================================================
#  部署
# ============================================================================

# 内部目标：执行部署
.PHONY: _deploy
_deploy: _cross
	@echo -e "$(CYAN)[deploy]$(RESET) 部署到 $(RKNN_HOST):$(RKNN_DEPLOY_PATH)..."
	@ssh -p $(RKNN_SSH_PORT) $(RKNN_HOST) "mkdir -p $(RKNN_DEPLOY_PATH)/bin"
	@scp -P $(RKNN_SSH_PORT) \
		$(WORKSPACE_ROOT)/target/$(RKNN_TARGET)/release/heimdall \
		$(RKNN_HOST):$(RKNN_DEPLOY_PATH)/bin/
	@echo -e "$(GREEN)[deploy]$(RESET) 部署完成: $(RKNN_HOST):$(RKNN_DEPLOY_PATH)/bin/heimdall"

# 用户命令
.PHONY: deploy
deploy: _deploy ## 部署到设备 (默认 RK3576)

.PHONY: deploy-rk3576
deploy-rk3576: ## 部署 RK3576 版本
	$(MAKE) _deploy RKNN_DEVICE=rk3576

.PHONY: deploy-rk3568
deploy-rk3568: ## 部署 RK3568 版本
	$(MAKE) _deploy RKNN_DEVICE=rk3568

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
clean: ## 清理所有构建产物
	$(CARGO) clean
	@echo -e "$(GREEN)[clean]$(RESET) 已清理全部构建产物"

.PHONY: clean-target
clean-target: ## 仅清理交叉编译产物
	@rm -rf $(WORKSPACE_ROOT)/target/$(RKNN_TARGET)
	@echo -e "$(GREEN)[clean-target]$(RESET) 已清理 $(RKNN_TARGET) 产物"

# ============================================================================
#  开发辅助
# ============================================================================

.PHONY: stats
stats: ## 查看项目代码统计
	@echo -e "$(CYAN)[stats]$(RESET) Rust 代码行数:"
	@find $(WORKSPACE_ROOT)/crates -name "*.rs" ! -path "*/target/*" | xargs wc -l | tail -1

.PHONY: tree-sdk
tree-sdk: ## 查看 SDK 库目录结构
	@echo -e "$(CYAN)SDK 库目录结构:$(RESET)"
	@find $(SDK_LIBS_DIR) -type f | sort | sed 's|$(SDK_LIBS_DIR)/||'
