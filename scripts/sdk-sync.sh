#!/usr/bin/env bash
# ============================================================================
#  Heimdall — 从目标设备同步 Rockchip SDK 库文件
# ============================================================================
#
#  用法:
#    ./scripts/sdk-sync.sh                           # 使用默认配置
#    ./scripts/sdk-sync.sh --host root@192.168.1.100 # 指定设备
#    ./scripts/sdk-sync.sh --device rk3568           # 指定目标设备型号
#
#  说明:
#    从目标 RK 设备提取交叉编译所需的最小库文件集合，
#    存储到 .rk-sdk-libs/<device>/ 目录供后续交叉编译使用。
#
# ============================================================================

set -eo pipefail

# ──────────────────────────────────────────────────────────────────────────────
# 默认配置
# ──────────────────────────────────────────────────────────────────────────────
WORKSPACE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SDK_LIBS_DIR="${WORKSPACE_ROOT}/.rk-sdk-libs"

# 默认设备配置（可通过参数覆盖）
RKNN_HOST="${RKNN_HOST:-root@192.168.1.100}"
RKNN_DEVICE="${RKNN_DEVICE:-rk3576}"
RKNN_SSH_PORT="${RKNN_SSH_PORT:-22}"

# 设备上的库文件路径
DEVICE_LIB_PATH="/usr/lib/aarch64-linux-gnu"

# 需要提取的最小库文件列表
REQUIRED_LIBS=(
    "librockchip_mpp.so"   # MPP 硬件解码
    "librga.so"            # RGA 2D 加速
    "librknnrt.so"         # RKNN Runtime
)

# ──────────────────────────────────────────────────────────────────────────────
# 颜色输出
# ──────────────────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
RESET='\033[0m'

info()  { echo -e "${CYAN}[INFO]${RESET}  $*"; }
ok()    { echo -e "${GREEN}[  OK]${RESET}  $*"; }
warn()  { echo -e "${YELLOW}[WARN]${RESET}  $*"; }
fail()  { echo -e "${RED}[FAIL]${RESET}  $*"; }
step()  { echo -e "\n${BOLD}── $* ──${RESET}"; }

# ──────────────────────────────────────────────────────────────────────────────
# 参数解析
# ──────────────────────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --host)
            RKNN_HOST="$2"
            shift 2
            ;;
        --device)
            RKNN_DEVICE="$2"
            shift 2
            ;;
        --port)
            RKNN_SSH_PORT="$2"
            shift 2
            ;;
        -h|--help)
            echo "用法: $0 [OPTIONS]"
            echo ""
            echo "选项:"
            echo "  --host HOST     设备 SSH 地址 (默认: root@192.168.1.100)"
            echo "  --device DEVICE 目标设备型号 (默认: rk3576)"
            echo "  --port PORT     SSH 端口 (默认: 22)"
            echo ""
            echo "示例:"
            echo "  $0 --host root@192.168.1.100 --device rk3576"
            echo ""
            exit 0
            ;;
        *)
            fail "未知参数: $1"
            exit 1
            ;;
    esac
done

# ──────────────────────────────────────────────────────────────────────────────
# 前置检查
# ──────────────────────────────────────────────────────────────────────────────
step "1. 环境检查"

# 检查 ssh
if ! command -v ssh &>/dev/null; then
    fail "ssh 未安装，请先安装 OpenSSH"
    exit 1
fi
ok "ssh 已就绪"

# 检查 scp
if ! command -v scp &>/dev/null; then
    fail "scp 未安装，请先安装 OpenSSH"
    exit 1
fi
ok "scp 已就绪"

# ──────────────────────────────────────────────────────────────────────────────
# 连接测试
# ──────────────────────────────────────────────────────────────────────────────
step "2. 连接测试"

info "测试连接: ${RKNN_HOST}:${RKNN_SSH_PORT}"
if ! ssh -p "${RKNN_SSH_PORT}" -o ConnectTimeout=5 "${RKNN_HOST}" "echo ok" &>/dev/null; then
    fail "无法连接到设备 ${RKNN_HOST}:${RKNN_SSH_PORT}"
    fail "请检查:"
    fail "  1. 设备 IP 是否正确"
    fail "  2. SSH 服务是否运行"
    fail "  3. 网络是否可达"
    exit 1
fi
ok "连接成功: ${RKNN_HOST}"

# ──────────────────────────────────────────────────────────────────────────────
# 检测设备型号
# ──────────────────────────────────────────────────────────────────────────────
step "3. 检测设备信息"

info "检测设备型号..."
DEVICE_MODEL=$(ssh -p "${RKNN_SSH_PORT}" "${RKNN_HOST}" "cat /proc/device-tree/model 2>/dev/null || echo unknown" | tr -d '\0')
ok "设备型号: ${DEVICE_MODEL}"

# ──────────────────────────────────────────────────────────────────────────────
# 检查库文件是否存在
# ──────────────────────────────────────────────────────────────────────────────
step "4. 检查设备库文件"

MISSING_LIBS=()
for lib in "${REQUIRED_LIBS[@]}"; do
    if ssh -p "${RKNN_SSH_PORT}" "${RKNN_HOST}" "test -f ${DEVICE_LIB_PATH}/${lib}" 2>/dev/null; then
        ok "${lib}"
    else
        warn "${lib} — 未找到"
        MISSING_LIBS+=("${lib}")
    fi
done

if [[ ${#MISSING_LIBS[@]} -gt 0 ]]; then
    echo ""
    warn "部分库文件未找到，但将继续同步已存在的文件"
    warn "缺失: ${MISSING_LIBS[*]}"
fi

# ──────────────────────────────────────────────────────────────────────────────
# 创建目标目录
# ──────────────────────────────────────────────────────────────────────────────
step "5. 同步库文件"

TARGET_DIR="${SDK_LIBS_DIR}/${RKNN_DEVICE}"
mkdir -p "${TARGET_DIR}"
ok "目标目录: ${TARGET_DIR}"

# ──────────────────────────────────────────────────────────────────────────────
# 同步文件
# ──────────────────────────────────────────────────────────────────────────────
SYNCED=0
FAILED=0

for lib in "${REQUIRED_LIBS[@]}"; do
    info "同步 ${lib}..."
    if scp -P "${RKNN_SSH_PORT}" -q "${RKNN_HOST}:${DEVICE_LIB_PATH}/${lib}" "${TARGET_DIR}/" 2>/dev/null; then
        # 获取文件大小
        SIZE=$(stat -f%z "${TARGET_DIR}/${lib}" 2>/dev/null || stat -c%s "${TARGET_DIR}/${lib}" 2>/dev/null || echo "unknown")
        ok "${lib} (${SIZE} bytes)"
        ((SYNCED++))
    else
        warn "${lib} — 同步失败"
        ((FAILED++))
    fi
done

# ──────────────────────────────────────────────────────────────────────────────
# 结果汇总
# ──────────────────────────────────────────────────────────────────────────────
step "6. 完成"

echo ""
echo -e "  设备型号: ${BOLD}${DEVICE_MODEL}${RESET}"
echo -e "  设备地址: ${BOLD}${RKNN_HOST}:${RKNN_SSH_PORT}${RESET}"
echo -e "  目标目录: ${BOLD}${TARGET_DIR}${RESET}"
echo ""
echo -e "  同步成功: ${GREEN}${SYNCED}${RESET}"
echo -e "  同步失败: ${RED}${FAILED}${RESET}"
echo ""

if [[ ${SYNCED} -eq 0 ]]; then
    fail "没有成功同步任何库文件"
    exit 1
fi

ok "${BOLD}SDK 库文件同步完成！${RESET}"
echo ""
echo -e "  ${CYAN}下一步:${RESET}"
echo "    make cross              # 交叉编译（默认 RK3576）"
echo "    make cross-rk3568       # 交叉编译 RK3568 版本"
echo ""
