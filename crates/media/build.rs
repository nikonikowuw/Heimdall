fn main() {
    #[cfg(feature = "mpp")]
    {
        // Rockchip MPP (Media Process Platform) 硬件解码库
        // 提供 mpp_create / mpp_init / mpp_destroy / mpp_packet_* / mpp_frame_* / mpp_buffer_* 等符号
        //
        // 库搜索路径优先级：
        //   1. RK_MPP_LIB_DIR 环境变量（交叉编译时指向 RK SDK sysroot 的 lib 目录）
        //   2. 系统默认搜索路径（本机构建时 librockchip_mpp.so 已在 /usr/lib 下）
        if let Ok(lib_dir) = std::env::var("RK_MPP_LIB_DIR") {
            println!("cargo:rustc-link-search=native={lib_dir}");
        }
        println!("cargo:rustc-link-lib=rockchip_mpp");
    }

    #[cfg(feature = "dvpp")]
    {
        // 华为昇腾 DVPP (Digital Video Pre-Processing) 硬件编解码与图像处理库
        // 提供 acldvpp* 系列符号
        if let Ok(lib_dir) = std::env::var("ASCEND_LIB_DIR") {
            println!("cargo:rustc-link-search=native={lib_dir}");
        }
        println!("cargo:rustc-link-lib=ascendcl");
        println!("cargo:rustc-link-lib=acldvpp");
    }
}
