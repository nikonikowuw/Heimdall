#[cfg(target_os = "linux")]
use std::fs;

/// 从系统 API 读取并解析系统信息，返回 owned 数据
pub fn read_system_info() -> SystemInfoRaw {
    SystemInfoRaw {
        cpu: read_cpu_usage(),
        memory: read_memory_info(),
        disk: read_disk_info(),
        uptime: read_uptime(),
        device_model: read_device_model(),
        os_info: read_os_info(),
        kernel_version: read_kernel_version(),
    }
}

#[derive(Debug)]
pub struct SystemInfoRaw {
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub disk: DiskInfo,
    pub uptime: f64,
    pub device_model: String,
    pub os_info: String,
    pub kernel_version: String,
}

#[derive(Debug)]
pub struct CpuInfo {
    /// CPU 使用率百分比 (0.0 - 100.0)
    pub usage_percent: f64,
}

#[derive(Debug)]
pub struct MemoryInfo {
    pub total_kb: u64,
    pub available_kb: u64,
}

#[derive(Debug)]
pub struct DiskInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

// ═══════════════════════════════════════════════════════════════════
// 磁盘信息 —— POSIX statvfs，跨平台通用
// ═══════════════════════════════════════════════════════════════════

fn read_disk_info() -> DiskInfo {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        let path = CString::new("/").expect("path should not contain NUL");
        // SAFETY: statvfs is a valid zeroable struct
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: statvfs is a POSIX API; the path is a valid C string and stat is zeroed
        if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } == 0 {
            // macOS HFS+ 上 f_frsize 可能为 0，应回退到 f_bsize
            let block_size = {
                let frsize = stat.f_frsize as u64;
                if frsize > 0 {
                    frsize
                } else {
                    stat.f_bsize as u64
                }
            };
            let total = block_size.saturating_mul(stat.f_blocks as u64);
            let available = block_size.saturating_mul(stat.f_bavail as u64);
            return DiskInfo {
                total_bytes: total,
                available_bytes: available,
            };
        }
    }
    DiskInfo {
        total_bytes: 0,
        available_bytes: 0,
    }
}

// ═══════════════════════════════════════════════════════════════════
// Linux 实现：读取 /proc 文件系统
// ═══════════════════════════════════════════════════════════════════

#[cfg(target_os = "linux")]
fn read_cpu_usage() -> CpuInfo {
    let sample1 = read_cpu_stat_values();
    // 在阻塞线程中休眠——此函数已通过 spawn_blocking 在专用线程执行
    std::thread::sleep(std::time::Duration::from_millis(200));
    let sample2 = read_cpu_stat_values();

    match (sample1, sample2) {
        (Some(s1), Some(s2)) => {
            let total_delta = s2.0.saturating_sub(s1.0);
            let idle_delta = s2.1.saturating_sub(s1.1);
            if total_delta > 0 {
                let usage = ((total_delta - idle_delta) as f64 / total_delta as f64) * 100.0;
                CpuInfo {
                    usage_percent: usage.clamp(0.0, 100.0),
                }
            } else {
                CpuInfo { usage_percent: 0.0 }
            }
        }
        _ => CpuInfo { usage_percent: 0.0 },
    }
}

/// 从 /proc/stat 读取 CPU 累计时间值：(total, idle)
#[cfg(target_os = "linux")]
fn read_cpu_stat_values() -> Option<(u64, u64)> {
    let content = fs::read_to_string("/proc/stat").ok()?;
    let first_line = content.lines().next()?;
    // cpu  user nice system idle iowait irq softirq steal
    let parts: Vec<u64> = first_line
        .split_whitespace()
        .skip(1)
        .filter_map(|s| s.parse().ok())
        .collect();

    if parts.len() >= 4 {
        let idle = parts[3];
        let total: u64 = parts.iter().sum();
        Some((total, idle))
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn read_memory_info() -> MemoryInfo {
    let content = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut total_kb = 0u64;
    let mut available_kb = 0u64;

    for line in content.lines() {
        if let Some(value) = line.strip_prefix("MemTotal:") {
            total_kb = parse_kb_value(value);
        } else if let Some(value) = line.strip_prefix("MemAvailable:") {
            available_kb = parse_kb_value(value);
        }
        if total_kb > 0 && available_kb > 0 {
            break;
        }
    }

    MemoryInfo {
        total_kb,
        available_kb,
    }
}

#[cfg(target_os = "linux")]
fn parse_kb_value(s: &str) -> u64 {
    s.split_whitespace()
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

#[cfg(target_os = "linux")]
fn read_uptime() -> f64 {
    fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|content| {
            content
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0.0)
}

#[cfg(target_os = "linux")]
fn read_device_model() -> String {
    // 优先读取 ARM 设备树模型 (Rockchip RK3588, Ascend Atlas, Raspberry Pi 等)
    fs::read_to_string("/proc/device-tree/model")
        .or_else(|_| fs::read_to_string("/sys/firmware/devicetree/base/model"))
        // 回退读取 x86 DMI/SMBIOS 虚拟节点
        .or_else(|_| fs::read_to_string("/sys/devices/virtual/dmi/id/board_name"))
        .or_else(|_| fs::read_to_string("/sys/devices/virtual/dmi/id/product_name"))
        .map(|s| s.trim_matches('\0').trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Unknown".to_string())
}

#[cfg(target_os = "linux")]
fn read_os_info() -> String {
    // 尝试 /etc/os-release
    if let Ok(content) = fs::read_to_string("/etc/os-release") {
        for line in content.lines() {
            if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
                return value.trim_matches('"').to_string();
            }
        }
    }
    // fallback: uname
    std::process::Command::new("uname")
        .arg("-sr")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

#[cfg(target_os = "linux")]
fn read_kernel_version() -> String {
    fs::read_to_string("/proc/version")
        .ok()
        .and_then(|content| {
            // "Linux version 5.10.160 ..."
            content.split_whitespace().nth(2).map(|s| s.to_string())
        })
        .or_else(|| {
            std::process::Command::new("uname")
                .arg("-r")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "Unknown".to_string())
}

// ═══════════════════════════════════════════════════════════════════
// macOS 实现：sysctl / getloadavg / sw_vers / uname
// ═══════════════════════════════════════════════════════════════════

/// 通过 `sysctlbyname` 读取 u64 样式的标量值（如 `hw.memsize`）
#[cfg(target_os = "macos")]
fn sysctlbyname_u64(name: &str) -> Option<u64> {
    use std::ffi::CString;

    let c_name = CString::new(name).ok()?;
    let mut value: u64 = 0;
    let mut size = std::mem::size_of::<u64>();

    // SAFETY: sysctlbyname 读取标量值，size 指向正确的缓冲区大小
    let ret = unsafe {
        libc::sysctlbyname(
            c_name.as_ptr(),
            &mut value as *mut u64 as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };

    if ret == 0 {
        Some(value)
    } else {
        None
    }
}

/// 通过 `sysctl` 读取字符串值
#[cfg(target_os = "macos")]
fn sysctl_read_string(mib: &[i32]) -> Option<String> {
    let mut size: usize = 0;
    // SAFETY: 首次调用获取所需缓冲区大小；oldp 为 null，oldlenp 指向有效 size 变量
    let ret = unsafe {
        libc::sysctl(
            mib.as_ptr() as *mut i32,
            mib.len() as u32,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if ret != 0 || size == 0 {
        return None;
    }

    let mut buf = vec![0u8; size];
    // SAFETY: buf 有正确的大小，sysctl 将写入 size 字节到 buf
    let ret = unsafe {
        libc::sysctl(
            mib.as_ptr() as *mut i32,
            mib.len() as u32,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if ret != 0 {
        return None;
    }

    // sysctl 返回的字符串可能以 NUL 结尾
    if let Some(last) = buf.last() {
        if *last == 0 {
            buf.pop();
        }
    }

    String::from_utf8(buf).ok()
}

/// macOS CPU 使用率：Mach `host_processor_info` 获取瞬时 CPU tick 采样
///
/// 与 Linux `/proc/stat` 等价——返回每个 CPU 的 user/system/idle/nice 累计 tick，
/// 两次采样的差值即为该时间窗口内的真实 CPU 占用百分比，无 `getloadavg`
/// 那样的指数平滑衰减问题。
///
/// `PROCESSOR_CPU_LOAD_INFO`（flavor = 2）返回 `4 × num_cpus` 个 `integer_t`：
///   `[user₀, system₀, idle₀, nice₀, user₁, system₁, idle₁, nice₁, ...]`
#[cfg(target_os = "macos")]
fn sample_macos_cpu_ticks() -> Option<(u64, u64)> {
    #[allow(non_camel_case_types)]
    type processor_info_array_t = *mut libc::integer_t;

    // SAFETY: C ABI 函数签名与 macOS mach/host_priv.h 一致。
    extern "C" {
        fn mach_host_self() -> libc::mach_port_t;
        fn host_processor_info(
            host: libc::host_t,
            flavor: libc::processor_flavor_t,
            out_processor_count: *mut libc::natural_t,
            out_processor_info: *mut processor_info_array_t,
            out_processor_info_cnt: *mut libc::mach_msg_type_number_t,
        ) -> libc::kern_return_t;
    }

    const KERN_SUCCESS: libc::kern_return_t = 0;
    const PROCESSOR_CPU_LOAD_INFO: libc::processor_flavor_t = 2;
    const CPU_STATE_MAX: usize = 4; // user, system, idle, nice

    // SAFETY: host_processor_info 由内核填充 CPU 计数和 tick 数组；复制数据后立即释放
    // Mach 分配的缓冲区，指针和长度均来自同一次成功的 API 调用。
    unsafe {
        let host = mach_host_self();
        let mut cpu_count: libc::natural_t = 0;
        let mut info_ptr: processor_info_array_t = std::ptr::null_mut();
        let mut info_count: libc::mach_msg_type_number_t = 0;

        let kr = host_processor_info(
            host,
            PROCESSOR_CPU_LOAD_INFO,
            &mut cpu_count,
            &mut info_ptr,
            &mut info_count,
        );
        if kr != KERN_SUCCESS || info_ptr.is_null() || cpu_count == 0 {
            return None;
        }

        let value_count = info_count as usize;
        let expected_count = (cpu_count as usize).checked_mul(CPU_STATE_MAX)?;
        if value_count < expected_count {
            #[allow(deprecated)]
            let _ = libc::vm_deallocate(
                libc::mach_task_self(),
                info_ptr as libc::vm_address_t,
                (value_count * std::mem::size_of::<libc::integer_t>()) as libc::vm_size_t,
            );
            return None;
        }

        // SAFETY: info_ptr 非空，expected_count 不超过内核返回的 info_count。
        let ticks = std::slice::from_raw_parts(info_ptr, expected_count).to_vec();

        #[allow(deprecated)]
        let _ = libc::vm_deallocate(
            libc::mach_task_self(),
            info_ptr as libc::vm_address_t,
            (value_count * std::mem::size_of::<libc::integer_t>()) as libc::vm_size_t,
        );

        let mut total_idle = 0u64;
        let mut total_all = 0u64;
        for cpu_idx in 0..cpu_count as usize {
            let base = cpu_idx * CPU_STATE_MAX;
            let user = u64::try_from(ticks[base]).ok()?;
            let system = u64::try_from(ticks[base + 1]).ok()?;
            let idle = u64::try_from(ticks[base + 2]).ok()?;
            let nice = u64::try_from(ticks[base + 3]).ok()?;
            total_idle = total_idle.saturating_add(idle);
            total_all = total_all
                .saturating_add(user)
                .saturating_add(system)
                .saturating_add(idle)
                .saturating_add(nice);
        }

        Some((total_all, total_idle))
    }
}

#[cfg(target_os = "macos")]
fn read_cpu_usage() -> CpuInfo {
    let sample1 = sample_macos_cpu_ticks();
    // 在阻塞线程中休眠——此函数已通过 spawn_blocking 在专用线程执行
    std::thread::sleep(std::time::Duration::from_millis(200));
    let sample2 = sample_macos_cpu_ticks();

    match (sample1, sample2) {
        (Some((total1, idle1)), Some((total2, idle2))) => {
            let total_delta = total2.saturating_sub(total1);
            let idle_delta = idle2.saturating_sub(idle1);
            if total_delta > 0 {
                let busy_delta = total_delta.saturating_sub(idle_delta);
                let usage_pct = (busy_delta as f64 / total_delta as f64) * 100.0;
                CpuInfo {
                    usage_percent: usage_pct.clamp(0.0, 100.0),
                }
            } else {
                CpuInfo { usage_percent: 0.0 }
            }
        }
        _ => CpuInfo { usage_percent: 0.0 },
    }
}

/// macOS 内存信息：`sysctl` 读取总量 + `vm_stat` 命令读取页面统计
#[cfg(target_os = "macos")]
fn read_memory_info() -> MemoryInfo {
    // 总物理内存（字节）
    let total_bytes = sysctlbyname_u64("hw.memsize").unwrap_or(0);
    let total_kb = total_bytes / 1024;

    // 通过 vm_stat 命令读取页面统计（Mach API 需要大量 FFI 类型定义，
    // vm_stat 命令是最实用的跨版本兼容方案）
    let available_kb = read_memory_available_vmstat();

    MemoryInfo {
        total_kb,
        available_kb,
    }
}

/// 解析 `vm_stat` 命令输出，累加可用页面数 × 页大小 = 可用内存（KB）
///
/// `vm_stat` 输出示例：
/// ```text
/// Pages free:                                834217.
/// Pages active:                             1234567.
/// Pages inactive:                            567890.
/// Pages speculative:                          12345.
/// Pages wired down:                           45678.
/// ...
/// ```
///
/// 可用内存 = (free + inactive + speculative) × page_size
#[cfg(target_os = "macos")]
fn read_memory_available_vmstat() -> u64 {
    // 默认页大小 16384 (Apple Silicon) / 4096 (Intel)，先尝试获取真实值
    let page_size = std::env::var("PAGE_SIZE")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(16384);

    let output = std::process::Command::new("vm_stat")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();

    let mut free_pages = 0u64;
    let mut inactive_pages = 0u64;
    let mut speculative_pages = 0u64;

    for line in output.lines() {
        let lower = line.to_lowercase();
        if let Some(val) = extract_page_count(line) {
            if lower.contains("pages free") {
                free_pages = val;
            } else if lower.contains("pages inactive") {
                inactive_pages = val;
            } else if lower.contains("pages speculative") {
                speculative_pages = val;
            }
        }
    }

    (free_pages + inactive_pages + speculative_pages) * page_size / 1024
}

/// 从 `vm_stat` 输出行中提取页面数（如 `"Pages free:    834217."` → `834217`）
#[cfg(target_os = "macos")]
fn extract_page_count(line: &str) -> Option<u64> {
    line.split(':')
        .nth(1)?
        .trim()
        .trim_end_matches('.')
        .trim()
        .parse()
        .ok()
}

/// 读取 macOS 启动时间的 Unix 秒时间戳。
#[cfg(target_os = "macos")]
fn read_macos_boot_secs() -> Option<u64> {
    let c_name = std::ffi::CString::new("kern.boottime").ok()?;
    // SAFETY: timeval 只包含整数标量，且这里仅用于接收 kern.boottime 的完整 C ABI 布局。
    let mut boot_time: libc::timeval = unsafe { std::mem::zeroed() };
    // kern.boottime 返回完整 timeval；在 64 位 macOS 上其大小为 16 字节。
    let mut size = std::mem::size_of::<libc::timeval>();
    // SAFETY: boot_time 是与 kern.boottime ABI 匹配的 timeval，size 是其完整布局大小。
    let ret = unsafe {
        libc::sysctlbyname(
            c_name.as_ptr(),
            &mut boot_time as *mut libc::timeval as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if ret != 0 || size < std::mem::size_of::<libc::time_t>() {
        return None;
    }

    u64::try_from(boot_time.tv_sec).ok()
}

/// macOS 运行时间：`sysctl kern.boottime` 获取启动时间戳后计算持续时长。
#[cfg(target_os = "macos")]
fn read_uptime() -> f64 {
    let Some(boot_secs) = read_macos_boot_secs() else {
        return 0.0;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    now.saturating_sub(boot_secs) as f64
}

/// macOS 设备型号：`sysctl hw.model`
#[cfg(target_os = "macos")]
fn read_device_model() -> String {
    sysctl_read_string(&[libc::CTL_HW, libc::HW_MODEL]).unwrap_or_else(|| "Unknown".to_string())
}

/// macOS 操作系统信息：`sw_vers -productName -productVersion`
#[cfg(target_os = "macos")]
fn read_os_info() -> String {
    let product_name = std::process::Command::new("sw_vers")
        .arg("-productName")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "macOS".to_string());

    let product_version = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    if product_version.is_empty() {
        product_name
    } else {
        format!("{product_name} {product_version}")
    }
}

/// macOS 内核版本：`uname -r`
#[cfg(target_os = "macos")]
fn read_kernel_version() -> String {
    std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_info_values_are_finite_and_non_negative() {
        let info = read_system_info();
        assert!(info.cpu.usage_percent.is_finite());
        assert!((0.0..=100.0).contains(&info.cpu.usage_percent));
        assert!(info.uptime.is_finite());
        assert!(info.uptime >= 0.0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_cpu_sampler_returns_monotonic_ticks() {
        let first = sample_macos_cpu_ticks().expect("Mach CPU tick 采样失败");
        std::thread::sleep(std::time::Duration::from_millis(20));
        let second = sample_macos_cpu_ticks().expect("Mach CPU tick 二次采样失败");
        assert!(second.0 > first.0, "first={first:?}, second={second:?}");
        assert!(second.1 >= first.1, "first={first:?}, second={second:?}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_boot_time_is_read_as_a_timeval() {
        let boot_secs = read_macos_boot_secs().expect("读取 kern.boottime 失败");
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("系统时间早于 Unix epoch")
            .as_secs();
        assert!(boot_secs > 0);
        assert!(boot_secs <= now_secs);
        assert!(read_uptime() > 0.0);
        assert!(read_uptime() < now_secs as f64);
    }
}
