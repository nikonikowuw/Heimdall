#!/usr/bin/env bash
# ============================================================================
#  Heimdall — 交叉编译环境检测与一键安装
# ============================================================================
#
#  用法:
#    ./scripts/setup-cross-deps.sh              检测 + 交互式安装缺失依赖
#    ./scripts/setup-cross-deps.sh --check      仅检测，不安装
#    ./scripts/setup-cross-deps.sh --install    非交互式，自动安装所有缺失依赖
#    ./scripts/setup-cross-deps.sh --target rknn  指定目标平台 (默认 rknn)
#
#  支持平台:
#    - macOS arm64 (Apple Silicon)   → aarch64-unknown-linux-gnu
#    - macOS x64 (Intel)             → aarch64-unknown-linux-gnu
#    - Linux x64                     → aarch64-unknown-linux-gnu
#    - Linux arm64                   → aarch64-unknown-linux-gnu (本机)
#
# ============================================================================

set -eo pipefail

# ──────────────────────────────────────────────────────────────────────────────
# 加载 Rust/cargo 环境 (确保 ~/.cargo/bin 在 PATH 中)
# ──────────────────────────────────────────────────────────────────────────────
if [[ -f "$HOME/.cargo/env" ]]; then
    source "$HOME/.cargo/env"
elif [[ -d "$HOME/.cargo/bin" ]]; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi

# ──────────────────────────────────────────────────────────────────────────────
# 颜色
# ──────────────────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
RESET='\033[0m'

# ──────────────────────────────────────────────────────────────────────────────
# 工具函数
# ──────────────────────────────────────────────────────────────────────────────
info()  { echo -e "${CYAN}[INFO]${RESET}  $*"; }
ok()    { echo -e "${GREEN}[  OK]${RESET}  $*"; }
warn()  { echo -e "${YELLOW}[WARN]${RESET}  $*"; }
fail()  { echo -e "${RED}[FAIL]${RESET}  $*"; }
step()  { echo -e "\n${BOLD}── $* ──${RESET}"; }

# ──────────────────────────────────────────────────────────────────────────────
# 参数解析
# ──────────────────────────────────────────────────────────────────────────────
MODE="check-and-install"   # check | check-and-install | install
TARGET_PLATFORM="rknn"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --check)
            MODE="check"
            shift
            ;;
        --install)
            MODE="install"
            shift
            ;;
        --target)
            TARGET_PLATFORM="$2"
            shift 2
            ;;
        -h|--help)
            echo "用法: $0 [--check|--install] [--target rknn|ascend]"
            echo ""
            echo "选项:"
            echo "  --check         仅检测缺失依赖，不执行安装"
            echo "  --install       非交互式自动安装所有缺失依赖"
            echo "  --target PLATFORM   指定目标平台 (默认: rknn)"
            exit 0
            ;;
        *)
            fail "未知参数: $1"
            exit 1
            ;;
    esac
done

# ──────────────────────────────────────────────────────────────────────────────
# 系统检测
# ──────────────────────────────────────────────────────────────────────────────
step "1. 系统环境检测"

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Darwin)
        OS_NAME="macOS"
        OS_VERSION="$(sw_vers -productVersion 2>/dev/null || echo unknown)"
        PKG_MGR="brew"
        ;;
    Linux)
        OS_NAME="Linux"
        if [[ -f /etc/os-release ]]; then
            OS_VERSION="$(. /etc/os-release && echo "$PRETTY_NAME")"
        else
            OS_VERSION="$(uname -r)"
        fi
        # 检测包管理器
        if command -v apt &>/dev/null; then
            PKG_MGR="apt"
        elif command -v dnf &>/dev/null; then
            PKG_MGR="dnf"
        elif command -v yum &>/dev/null; then
            PKG_MGR="yum"
        elif command -v pacman &>/dev/null; then
            PKG_MGR="pacman"
        else
            PKG_MGR="unknown"
        fi
        ;;
    *)
        fail "不支持的操作系统: $OS"
        exit 1
        ;;
esac

case "$ARCH" in
    arm64|aarch64)  HOST_ARCH="aarch64" ;;
    x86_64|amd64)   HOST_ARCH="x86_64" ;;
    *)              HOST_ARCH="$ARCH" ;;
esac

# 目标交叉编译 triple
CROSS_TARGET="aarch64-unknown-linux-gnu"

ok "操作系统:   ${BOLD}$OS_NAME $OS_VERSION${RESET} ($ARCH)"
ok "包管理器:   ${BOLD}$PKG_MGR${RESET}"
ok "交叉编译:   ${BOLD}$HOST_ARCH-$OS_NAME${RESET} → ${BOLD}$CROSS_TARGET${RESET}"
ok "目标平台:   ${BOLD}$TARGET_PLATFORM${RESET}"

# ──────────────────────────────────────────────────────────────────────────────
# 依赖检测
# ──────────────────────────────────────────────────────────────────────────────
step "2. 依赖项检测"

MISSING=()
INSTALLED=()

check_tool() {
    local name="$1"
    local cmd="$2"
    local hint="$3"

    if eval "$cmd" &>/dev/null; then
        local version
        version=$(eval "$3" 2>/dev/null || echo "ok")
        ok "$name ${DIM}($version)${RESET}"
        INSTALLED+=("$name")
    else
        fail "$name ${DIM}— 未安装${RESET}"
        MISSING+=("$name")
    fi
}

# --- 必需依赖 ---

info "必需依赖:"

# rustup
check_tool "rustup" \
    "command -v rustup" \
    "rustup --version 2>&1 | head -1"

# aarch64-unknown-linux-gnu target
if command -v rustup &>/dev/null; then
    if rustup target list --installed 2>/dev/null | grep -q "aarch64-unknown-linux-gnu"; then
        ok "rustup target: aarch64-unknown-linux-gnu ${DIM}(已安装)${RESET}"
        INSTALLED+=("rustup-target")
    else
        fail "rustup target: aarch64-unknown-linux-gnu ${DIM}— 未安装${RESET}"
        MISSING+=("rustup-target")
    fi
else
    fail "rustup target: aarch64-unknown-linux-gnu ${DIM}— rustup 未安装，跳过检测${RESET}"
    MISSING+=("rustup-target")
fi

# cargo-zigbuild
check_tool "cargo-zigbuild" \
    "command -v cargo-zigbuild" \
    "cargo-zigbuild --version 2>&1"

# zig
check_tool "zig" \
    "command -v zig" \
    "zig version 2>&1"

# ssh (用于部署)
if command -v ssh &>/dev/null; then
    ok "ssh ${DIM}(部署用)${RESET}"
else
    warn "ssh ${DIM}— 未安装 (仅部署需要)${RESET}"
fi

# ──────────────────────────────────────────────────────────────────────────────
# 结果汇总
# ──────────────────────────────────────────────────────────────────────────────
step "3. 检测结果"

TOTAL=$((${#INSTALLED[@]} + ${#MISSING[@]}))
echo ""
echo -e "  已安装: ${GREEN}${#INSTALLED[@]}${RESET} / $TOTAL"
for tool in "${INSTALLED[@]}"; do
    echo -e "    ${GREEN}✓${RESET} $tool"
done

echo ""
echo -e "  缺失:   ${RED}${#MISSING[@]}${RESET} / $TOTAL"
for tool in "${MISSING[@]}"; do
    echo -e "    ${RED}✗${RESET} $tool"
done
echo ""

# ──────────────────────────────────────────────────────────────────────────────
# 安装逻辑
# ──────────────────────────────────────────────────────────────────────────────
if [[ ${#MISSING[@]} -eq 0 ]]; then
    ok "${BOLD}所有依赖已就绪！可以开始交叉编译:${RESET}"
    echo ""
    echo "    make rknn           # 交叉编译主程序"
    echo ""
    exit 0
fi

if [[ "$MODE" == "check" ]]; then
    warn "检测到缺失依赖，请手动安装或使用 --install 自动安装"
    echo ""
    exit 1
fi

# ──────────────────────────────────────────────────────────────────────────────
# 交互式确认
# ──────────────────────────────────────────────────────────────────────────────
if [[ "$MODE" == "check-and-install" ]]; then
    echo -e "${YELLOW}是否自动安装缺失依赖？${RESET}"
    echo ""
    read -r -p "  安装以上 ${#MISSING[@]} 个缺失工具? [y/N] " REPLY
    echo ""
    if [[ ! "$REPLY" =~ ^[Yy]$ ]]; then
        info "已取消安装。可稍后运行: $0 --install"
        exit 0
    fi
fi

# ──────────────────────────────────────────────────────────────────────────────
# 安装 rustup
# ──────────────────────────────────────────────────────────────────────────────
install_rustup() {
    step "安装 rustup"
    info "通过官方安装脚本安装 rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    # 加载 rustup 环境
    export PATH="$HOME/.cargo/bin:$PATH"
    source "$HOME/.cargo/env" 2>/dev/null || true

    if command -v rustup &>/dev/null; then
        ok "rustup 安装完成: $(rustup --version | head -1)"
    else
        fail "rustup 安装失败"
        exit 1
    fi
}

# ──────────────────────────────────────────────────────────────────────────────
# 安装 rustup target
# ──────────────────────────────────────────────────────────────────────────────
install_rustup_target() {
    step "安装 rustup target: $CROSS_TARGET"
    rustup target add "$CROSS_TARGET"
    ok "target $CROSS_TARGET 已安装"
}

# ──────────────────────────────────────────────────────────────────────────────
# 安装 cargo-zigbuild
# ──────────────────────────────────────────────────────────────────────────────
install_cargo_zigbuild() {
    step "安装 cargo-zigbuild"
    info "通过 cargo install 编译安装 cargo-zigbuild..."
    cargo install cargo-zigbuild

    if command -v cargo-zigbuild &>/dev/null; then
        ok "cargo-zigbuild 安装完成: $(cargo-zigbuild --version 2>&1)"
    else
        fail "cargo-zigbuild 安装失败"
        exit 1
    fi
}

# ──────────────────────────────────────────────────────────────────────────────
# 安装 zig
# ──────────────────────────────────────────────────────────────────────────────
install_zig() {
    step "安装 zig"
    case "$OS_NAME" in
        macOS)
            if command -v brew &>/dev/null; then
                info "通过 Homebrew 安装 zig..."
                brew install zig
            else
                fail "macOS 需要 Homebrew 安装 zig"
                fail "请先安装 Homebrew: https://brew.sh"
                exit 1
            fi
            ;;
        Linux)
            if command -v apt &>/dev/null; then
                info "通过 apt 安装 zig..."
                # zig 官方 deb 包
                local ZIG_VERSION="0.16.0"
                local ZIG_ARCH
                case "$HOST_ARCH" in
                    aarch64) ZIG_ARCH="aarch64" ;;
                    x86_64)  ZIG_ARCH="x86_64" ;;
                    *)       ZIG_ARCH="$HOST_ARCH" ;;
                esac
                local ZIG_URL="https://ziglang.org/download/${ZIG_VERSION}/zig-linux-${ZIG_ARCH}-${ZIG_VERSION}.tar.xz"
                info "下载 zig ${ZIG_VERSION}..."
                curl -sL "$ZIG_URL" | tar -xJ -C /tmp/
                sudo mv "/tmp/zig-linux-${ZIG_ARCH}-${ZIG_VERSION}/zig" /usr/local/bin/zig
                rm -rf "/tmp/zig-linux-${ZIG_ARCH}-${ZIG_VERSION}"
            elif command -v snap &>/dev/null; then
                info "通过 snap 安装 zig..."
                sudo snap install zig --classic --beta
            else
                fail "Linux 请手动安装 zig: https://ziglang.org/download/"
                exit 1
            fi
            ;;
    esac

    if command -v zig &>/dev/null; then
        ok "zig 安装完成: $(zig version 2>&1)"
    else
        fail "zig 安装失败"
        exit 1
    fi
}

# ──────────────────────────────────────────────────────────────────────────────
# 执行安装
# ──────────────────────────────────────────────────────────────────────────────
step "4. 开始安装缺失依赖"

for tool in "${MISSING[@]}"; do
    case "$tool" in
        rustup)         install_rustup ;;
        rustup-target)  install_rustup_target ;;
        cargo-zigbuild) install_cargo_zigbuild ;;
        zig)            install_zig ;;
    esac
done

# ──────────────────────────────────────────────────────────────────────────────
# 最终验证
# ──────────────────────────────────────────────────────────────────────────────
step "5. 最终验证"

echo ""
PASS=true

# rustup
if command -v rustup &>/dev/null; then
    ok "rustup: $(rustup --version | head -1)"
else
    fail "rustup: 未安装"
    PASS=false
fi

# target
if command -v rustup &>/dev/null && rustup target list --installed 2>/dev/null | grep -q "aarch64-unknown-linux-gnu"; then
    ok "target: aarch64-unknown-linux-gnu"
else
    fail "target: aarch64-unknown-linux-gnu 未安装"
    PASS=false
fi

# cargo-zigbuild
if command -v cargo-zigbuild &>/dev/null; then
    ok "cargo-zigbuild: $(cargo-zigbuild --version 2>&1)"
else
    fail "cargo-zigbuild: 未安装"
    PASS=false
fi

# zig
if command -v zig &>/dev/null; then
    ok "zig: $(zig version 2>&1)"
else
    fail "zig: 未安装"
    PASS=false
fi

echo ""

if $PASS; then
    ok "${BOLD}🎉 所有交叉编译依赖已就绪！${RESET}"
    echo ""
    echo -e "  ${CYAN}快速开始:${RESET}"
    echo "    cd $(dirname "$0")/.."
    echo "    make verify-cross    # 验证工具链"
    echo "    make rknn            # 交叉编译主程序"
    echo "    make rknn-algo       # 交叉编译算法包"
    echo "    make rknn-package    # 交叉编译 + 打包"
    echo ""
else
    fail "${BOLD}部分依赖安装失败，请检查上方错误信息${RESET}"
    echo ""
    exit 1
fi
