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
RKNN_DEVICE=""
EXPLICIT_DEVICE=""
RKNN_SSH_PORT="${RKNN_SSH_PORT:-22}"

# 候选搜索目录列表（板端搜索）
SEARCH_DIRS="/usr/lib/aarch64-linux-gnu /usr/lib /usr/local/lib /usr/lib64 /lib/aarch64-linux-gnu /lib /vendor/lib64 /vendor/lib /oem/usr/lib"

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
            EXPLICIT_DEVICE="$2"
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

# 若传入纯 IP 地址且未指定用户名，默认使用 root 用户
if echo "${RKNN_HOST}" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$'; then
    RKNN_HOST="root@${RKNN_HOST}"
fi

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
# 检测设备型号与 SoC
# ──────────────────────────────────────────────────────────────────────────────
step "3. 检测设备信息"

detect_soc() {
    local text="$1"
    local lower
    lower=$(echo "$text" | tr '[:upper:]' '[:lower:]')
    case "$lower" in
        *rk3588*|*3588*) echo "rk3588" ;;
        *rk3576*|*3576*) echo "rk3576" ;;
        *rk3568*|*3568*) echo "rk3568" ;;
        *rk3566*|*3566*) echo "rk3566" ;;
        *rk3562*|*3562*) echo "rk3562" ;;
        *rv1126*|*1126*) echo "rv1126" ;;
        *rv1109*|*1109*) echo "rv1109" ;;
        *) echo "" ;;
    esac
}

info "检测设备型号..."
DEVICE_MODEL=$(ssh -p "${RKNN_SSH_PORT}" "${RKNN_HOST}" "cat /proc/device-tree/model 2>/dev/null || cat /sys/firmware/devicetree/base/model 2>/dev/null || echo unknown" | tr -d '\0')
DEVICE_COMPAT=$(ssh -p "${RKNN_SSH_PORT}" "${RKNN_HOST}" "cat /proc/device-tree/compatible 2>/dev/null | tr '\0' ' ' || true")
ok "设备型号: ${DEVICE_MODEL}"

DETECTED_SOC=$(detect_soc "${DEVICE_MODEL} ${DEVICE_COMPAT}")
if [ -n "${DETECTED_SOC}" ]; then
    ok "识别芯片: ${DETECTED_SOC}"
fi

# 确定存储子目录：若未显式指定则优先使用检测到的 SoC 型号
if [ -n "${EXPLICIT_DEVICE}" ] && [ "${EXPLICIT_DEVICE}" != "auto" ]; then
    RKNN_DEVICE="${EXPLICIT_DEVICE}"
    if [ -n "${DETECTED_SOC}" ] && [ "${DETECTED_SOC}" != "${EXPLICIT_DEVICE}" ]; then
        warn "指定的型号 [${EXPLICIT_DEVICE}] 与硬件识别型号 [${DETECTED_SOC}] 不一致！"
    fi
elif [ -n "${DETECTED_SOC}" ]; then
    RKNN_DEVICE="${DETECTED_SOC}"
    ok "目标目录使用自动检测型号: ${RKNN_DEVICE}"
else
    RKNN_DEVICE="rk3576"
    warn "未能自动检测 SoC 型号，回退至默认配置: ${RKNN_DEVICE}"
fi

# ──────────────────────────────────────────────────────────────────────────────
# 检查库文件是否存在
# ──────────────────────────────────────────────────────────────────────────────
step "4. 检查设备库文件"

REMOTE_CHECK_SCRIPT="
for lib in ${REQUIRED_LIBS[*]}; do
    found=\"\"
    for dir in ${SEARCH_DIRS}; do
        if [ -e \"\$dir/\$lib\" ]; then
            found=\"\$dir/\$lib\"
            break
        fi
    done
    if [ -z \"\$found\" ]; then
        found=\$(find /usr/lib /usr/local/lib /lib /vendor /oem -name \"\$lib\" 2>/dev/null | head -n 1)
    fi
    if [ -n \"\$found\" ]; then
        echo \"\$lib:\$found\"
    else
        echo \"\$lib:NOT_FOUND\"
    fi
done
"

REMOTE_RESULT=$(ssh -p "${RKNN_SSH_PORT}" "${RKNN_HOST}" "${REMOTE_CHECK_SCRIPT}")

FOUND_LIBS=()
FOUND_PATHS=()
MISSING_LIBS=()

while IFS=":" read -r lib_name lib_path; do
    [ -z "$lib_name" ] && continue
    if [ "$lib_path" = "NOT_FOUND" ]; then
        warn "${lib_name} — 未找到"
        MISSING_LIBS=("${MISSING_LIBS[@]}" "$lib_name")
    else
        ok "${lib_name} (${lib_path})"
        FOUND_LIBS=("${FOUND_LIBS[@]}" "$lib_name")
        FOUND_PATHS=("${FOUND_PATHS[@]}" "$lib_path")
    fi
done <<< "${REMOTE_RESULT}"

if [ ${#MISSING_LIBS[@]} -gt 0 ]; then
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

i=0
while [ $i -lt ${#FOUND_LIBS[@]} ]; do
    lib="${FOUND_LIBS[$i]}"
    rpath="${FOUND_PATHS[$i]}"
    info "同步 ${lib}..."
    if scp -P "${RKNN_SSH_PORT}" -q "${RKNN_HOST}:${rpath}" "${TARGET_DIR}/${lib}" 2>/dev/null; then
        SIZE=$(stat -f%z "${TARGET_DIR}/${lib}" 2>/dev/null || stat -c%s "${TARGET_DIR}/${lib}" 2>/dev/null || echo "unknown")
        ok "${lib} (${SIZE} bytes)"
        SYNCED=$((SYNCED + 1))
    else
        warn "${lib} — 同步失败"
        FAILED=$((FAILED + 1))
    fi
    i=$((i + 1))
done

FAILED=$((FAILED + ${#MISSING_LIBS[@]}))

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
if [ "${RKNN_DEVICE}" = "rk3568" ]; then
    echo "    make cross-rk3568       # 交叉编译 RK3568 版本"
    echo "    make deploy-rk3568      # 部署到 RK3568 设备"
elif [ "${RKNN_DEVICE}" = "rk3576" ]; then
    echo "    make cross              # 交叉编译（默认 RK3576）"
    echo "    make deploy             # 部署到 RK3576 设备"
else
    echo "    make cross RKNN_DEVICE=${RKNN_DEVICE}   # 交叉编译"
fi
echo ""
