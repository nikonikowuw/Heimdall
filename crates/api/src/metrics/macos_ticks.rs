//! macOS Mach `host_processor_info` CPU tick 采样
//!
//! 提供瞬时每核心 CPU tick 读取，用于精确计算 CPU 使用率（两次采样差值法）。

/// 每核心瞬时 tick 采样：返回 `Vec<(all_ticks, idle_ticks)>`
///
/// `PROCESSOR_CPU_LOAD_INFO`（flavor = 2）返回 `4 × num_cpus` 个 `integer_t`：
/// `[user₀, system₀, idle₀, nice₀, user₁, system₁, idle₁, nice₁, ...]`
#[cfg(target_os = "macos")]
pub fn sample_macos_per_core_ticks() -> Option<Vec<(u64, u64)>> {
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
    const CPU_STATE_MAX: usize = 4;

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

        let mut cores = Vec::with_capacity(cpu_count as usize);
        for cpu_idx in 0..cpu_count as usize {
            let base = cpu_idx * CPU_STATE_MAX;
            let user = u64::try_from(ticks[base]).ok()?;
            let system = u64::try_from(ticks[base + 1]).ok()?;
            let idle = u64::try_from(ticks[base + 2]).ok()?;
            let nice = u64::try_from(ticks[base + 3]).ok()?;
            let all = user
                .saturating_add(system)
                .saturating_add(idle)
                .saturating_add(nice);
            cores.push((all, idle));
        }

        Some(cores)
    }
}

/// macOS CPU 总体瞬时 tick 采样（汇总所有核心）
#[cfg(target_os = "macos")]
pub fn sample_macos_cpu_ticks() -> Option<(u64, u64)> {
    let cores = sample_macos_per_core_ticks()?;
    let mut total_all = 0u64;
    let mut total_idle = 0u64;
    for (all, idle) in cores {
        total_all = total_all.saturating_add(all);
        total_idle = total_idle.saturating_add(idle);
    }
    Some((total_all, total_idle))
}
