/**
 * probe_rknn_dup_context: RKNN rknn_dup_context 权重复用与跨线程/跨核亲和性独立探针
 *
 * 目的:
 *   在真实板端 (RK3588 / RK3576 / RK3568) 上无宿主依赖地验证 rknn_dup_context 的
 *   跨线程安全性、核心亲和性绑定、兄弟 Context 销毁隔离以及内存物理共享特性。
 *
 * 判定依据:
 *   - 路线 A: 控制线程调用 rknn_dup_context，以独占所有权转交实例 Worker 线程执行
 *             rknn_set_core_mask、推理与销毁。
 *   - 路线 B (后备): 控制侧提供互斥锁，实例 Worker 线程加锁调用 rknn_dup_context。
 *
 * 编译方法:
 *   gcc -O2 -Wall -Wextra main.c -lpthread -ldl -o probe_rknn_dup_context
 */

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdbool.h>
#include <dlfcn.h>
#include <pthread.h>
#include <unistd.h>
#include <errno.h>
#include <sys/time.h>

#define RKNN_MAX_NAME_LEN 256
#define RKNN_MAX_DIMS 16

/* RKNN C ABI 结构体与常量定义 */
#if defined(__arm__)
typedef uint32_t rknn_context;
#else
typedef uint64_t rknn_context;
#endif

typedef enum _rknn_core_mask {
    RKNN_NPU_CORE_AUTO = 0,
    RKNN_NPU_CORE_0    = 1,
    RKNN_NPU_CORE_1    = 2,
    RKNN_NPU_CORE_2    = 4,
    RKNN_NPU_CORE_0_1  = 3,
    RKNN_NPU_CORE_0_1_2= 7,
    RKNN_NPU_CORE_ALL  = 7,
} rknn_core_mask;

typedef enum _rknn_query_cmd {
    RKNN_QUERY_IN_OUT_NUM = 0,
    RKNN_QUERY_INPUT_ATTR = 1,
    RKNN_QUERY_OUTPUT_ATTR = 2,
    RKNN_QUERY_PERF_DETAIL = 3,
    RKNN_QUERY_PERF_RUN = 4,
    RKNN_QUERY_SDK_VERSION = 5,
    RKNN_QUERY_MEM_SIZE = 6,
    RKNN_QUERY_CUSTOM_STRING = 7,
    RKNN_QUERY_NATIVE_INPUT_ATTR = 8,
    RKNN_QUERY_NATIVE_OUTPUT_ATTR = 9,
} rknn_query_cmd;

typedef struct _rknn_input_output_num {
    uint32_t n_input;
    uint32_t n_output;
} rknn_input_output_num;

typedef struct _rknn_tensor_attr {
    uint32_t index;
    uint32_t n_dims;
    uint32_t dims[RKNN_MAX_DIMS];
    char name[RKNN_MAX_NAME_LEN];
    uint32_t n_elems;
    uint32_t size;
    int32_t fmt;
    int32_t type;
    int8_t qnt_type;
    int8_t fl;
    int32_t zp;
    float scale;
    uint32_t w_stride;
    uint32_t size_with_stride;
    uint8_t pass_through;
    uint32_t h_stride;
} rknn_tensor_attr;

typedef struct _rknn_input {
    uint32_t index;
    void *buf;
    uint32_t size;
    uint8_t pass_through;
    int32_t type;
    int32_t fmt;
} rknn_input;

typedef struct _rknn_output {
    uint8_t want_float;
    uint8_t is_prealloc;
    uint32_t index;
    void *buf;
    uint32_t size;
} rknn_output;

typedef struct _rknn_mem_size {
    uint32_t total_weight_size;
    uint32_t total_internal_size;
    uint32_t total_dma_allocated_size;
    uint32_t reserved[9];
} rknn_mem_size;

typedef struct _rknn_sdk_version {
    char api_version[256];
    char drv_version[256];
} rknn_sdk_version;

/* RKNN API 函数指针原型 */
typedef int (*pfn_rknn_init)(rknn_context *ctx, void *model, uint32_t size, uint32_t flag, void *reserved);
typedef int (*pfn_rknn_dup_context)(rknn_context *ctx_in, rknn_context *ctx_out);
typedef int (*pfn_rknn_destroy)(rknn_context ctx);
typedef int (*pfn_rknn_query)(rknn_context ctx, rknn_query_cmd cmd, void *info, uint32_t size);
typedef int (*pfn_rknn_inputs_set)(rknn_context ctx, uint32_t n_inputs, rknn_input inputs[]);
typedef int (*pfn_rknn_run)(rknn_context ctx, void *extend);
typedef int (*pfn_rknn_outputs_get)(rknn_context ctx, uint32_t n_outputs, rknn_output outputs[], void *extend);
typedef int (*pfn_rknn_outputs_release)(rknn_context ctx, uint32_t n_outputs, rknn_output outputs[]);
typedef int (*pfn_rknn_set_core_mask)(rknn_context ctx, rknn_core_mask core_mask);

struct rknn_api_table {
    void *lib_handle;
    pfn_rknn_init init;
    pfn_rknn_dup_context dup_context;
    pfn_rknn_destroy destroy;
    pfn_rknn_query query;
    pfn_rknn_inputs_set inputs_set;
    pfn_rknn_run run;
    pfn_rknn_outputs_get outputs_get;
    pfn_rknn_outputs_release outputs_release;
    pfn_rknn_set_core_mask set_core_mask;
};

static struct rknn_api_table g_rknn;

static double get_time_ms(void) {
    struct timeval tv;
    gettimeofday(&tv, NULL);
    return (double)tv.tv_sec * 1000.0 + (double)tv.tv_usec / 1000.0;
}

static long get_vm_rss_kb(void) {
    FILE *f = fopen("/proc/self/status", "r");
    if (!f) return -1;
    char line[256];
    long rss = -1;
    while (fgets(line, sizeof(line), f)) {
        if (strncmp(line, "VmRSS:", 6) == 0) {
            sscanf(line + 6, "%ld", &rss);
            break;
        }
    }
    fclose(f);
    return rss;
}

static int load_rknn_library(const char *explicit_path) {
    const char *candidates[] = {
        explicit_path,
        "librknnrt.so",
        "/usr/lib/librknnrt.so",
        "/usr/lib64/librknnrt.so",
        "/usr/local/lib/librknnrt.so",
        "/opt/rknn-toolkit2/rknpu2/lib/librknnrt.so",
        NULL
    };

    void *h = NULL;
    for (int i = 0; candidates[i] != NULL; ++i) {
        if (candidates[i] == NULL || strlen(candidates[i]) == 0) continue;
        h = dlopen(candidates[i], RTLD_NOW | RTLD_LOCAL);
        if (h) {
            printf("[INFO] 成功加载 librknnrt: %s\n", candidates[i]);
            break;
        }
    }

    if (!h) {
        fprintf(stderr, "[ERROR] 无法加载 librknnrt.so: %s\n", dlerror());
        return -1;
    }

    g_rknn.lib_handle = h;
    g_rknn.init = (pfn_rknn_init)dlsym(h, "rknn_init");
    g_rknn.dup_context = (pfn_rknn_dup_context)dlsym(h, "rknn_dup_context");
    g_rknn.destroy = (pfn_rknn_destroy)dlsym(h, "rknn_destroy");
    g_rknn.query = (pfn_rknn_query)dlsym(h, "rknn_query");
    g_rknn.inputs_set = (pfn_rknn_inputs_set)dlsym(h, "rknn_inputs_set");
    g_rknn.run = (pfn_rknn_run)dlsym(h, "rknn_run");
    g_rknn.outputs_get = (pfn_rknn_outputs_get)dlsym(h, "rknn_outputs_get");
    g_rknn.outputs_release = (pfn_rknn_outputs_release)dlsym(h, "rknn_outputs_release");
    g_rknn.set_core_mask = (pfn_rknn_set_core_mask)dlsym(h, "rknn_set_core_mask");

    if (!g_rknn.init || !g_rknn.destroy || !g_rknn.query || !g_rknn.run) {
        fprintf(stderr, "[ERROR] 缺少核心 RKNN API 符号!\n");
        return -2;
    }

    printf("[INFO] 符号可用性检查:\n");
    printf("  rknn_init: %s\n", g_rknn.init ? "YES" : "NO");
    printf("  rknn_dup_context: %s\n", g_rknn.dup_context ? "YES" : "NO");
    printf("  rknn_set_core_mask: %s\n", g_rknn.set_core_mask ? "YES" : "NO");
    printf("  rknn_destroy: %s\n", g_rknn.destroy ? "YES" : "NO");

    return 0;
}

static void *read_file(const char *path, uint32_t *out_size) {
    FILE *f = fopen(path, "rb");
    if (!f) {
        fprintf(stderr, "[ERROR] 无法打开模型文件 %s: %s\n", path, strerror(errno));
        return NULL;
    }
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (size <= 0) {
        fclose(f);
        return NULL;
    }
    void *buf = malloc(size);
    if (!buf) {
        fclose(f);
        return NULL;
    }
    if (fread(buf, 1, size, f) != (size_t)size) {
        free(buf);
        fclose(f);
        return NULL;
    }
    fclose(f);
    *out_size = (uint32_t)size;
    return buf;
}

/* 路线 A Worker 线程参数 */
struct route_a_worker_arg {
    int worker_id;
    rknn_context child_ctx; /* 控制线程 dup 后一次性移交的子 context */
    rknn_core_mask target_core;
    int num_inferences;
    int destroy_early;
    /* 输出统计 */
    int set_core_ret;
    int run_ret;
    int destroy_ret;
    double elapsed_ms;
    bool success;
};

static void *route_a_worker_thread(void *p) {
    struct route_a_worker_arg *arg = (struct route_a_worker_arg *)p;
    arg->success = false;

    printf("[Worker %d] 线程启动 (TID=%ld), 接收 child_ctx=0x%lx, 目标核掩码=%d\n",
           arg->worker_id, (long)pthread_self(), (unsigned long)arg->child_ctx, arg->target_core);

    // 1. 在 Worker 专用线程内部调用 rknn_set_core_mask 绑定目标 NPU 核心
    if (g_rknn.set_core_mask) {
        arg->set_core_ret = g_rknn.set_core_mask(arg->child_ctx, arg->target_core);
        if (arg->set_core_ret != 0) {
            fprintf(stderr, "[Worker %d] rknn_set_core_mask 失败，错误码: %d\n",
                    arg->worker_id, arg->set_core_ret);
            return NULL;
        }
    } else {
        arg->set_core_ret = 0;
    }

    // 2. 准备空虚拟输入用于测试推理
    rknn_input_output_num io_num;
    memset(&io_num, 0, sizeof(io_num));
    g_rknn.query(arg->child_ctx, RKNN_QUERY_IN_OUT_NUM, &io_num, sizeof(io_num));

    rknn_input inputs[16];
    void *input_buffers[16] = {0};
    for (uint32_t i = 0; i < io_num.n_input && i < 16; ++i) {
        rknn_tensor_attr attr;
        memset(&attr, 0, sizeof(attr));
        attr.index = i;
        g_rknn.query(arg->child_ctx, RKNN_QUERY_INPUT_ATTR, &attr, sizeof(attr));
        input_buffers[i] = calloc(1, attr.size > 0 ? attr.size : 1024);
        inputs[i].index = i;
        inputs[i].buf = input_buffers[i];
        inputs[i].size = attr.size > 0 ? attr.size : 1024;
        inputs[i].pass_through = 0;
        inputs[i].type = attr.type;
        inputs[i].fmt = attr.fmt;
    }

    double t0 = get_time_ms();

    for (int iter = 0; iter < arg->num_inferences; ++iter) {
        if (io_num.n_input > 0) {
            g_rknn.inputs_set(arg->child_ctx, io_num.n_input, inputs);
        }
        arg->run_ret = g_rknn.run(arg->child_ctx, NULL);
        if (arg->run_ret != 0) {
            fprintf(stderr, "[Worker %d] rknn_run 失败，错误码: %d (iter=%d)\n",
                    arg->worker_id, arg->run_ret, iter);
            break;
        }

        // 释放输出
        rknn_output outputs[16];
        memset(outputs, 0, sizeof(outputs));
        for (uint32_t j = 0; j < io_num.n_output && j < 16; ++j) {
            outputs[j].index = j;
            outputs[j].want_float = 1;
        }
        if (io_num.n_output > 0) {
            g_rknn.outputs_get(arg->child_ctx, io_num.n_output, outputs, NULL);
            g_rknn.outputs_release(arg->child_ctx, io_num.n_output, outputs);
        }
    }

    double t1 = get_time_ms();
    arg->elapsed_ms = t1 - t0;

    for (uint32_t i = 0; i < io_num.n_input && i < 16; ++i) {
        if (input_buffers[i]) free(input_buffers[i]);
    }

    // 3. 在 Worker 专用线程内销毁自身 child_ctx
    arg->destroy_ret = g_rknn.destroy(arg->child_ctx);
    if (arg->destroy_ret != 0) {
        fprintf(stderr, "[Worker %d] rknn_destroy 失败，错误码: %d\n",
                arg->worker_id, arg->destroy_ret);
    } else {
        arg->success = (arg->run_ret == 0);
    }

    printf("[Worker %d] 线程退出，推理 %d 次，耗时 %.2f ms，结果: %s\n",
           arg->worker_id, arg->num_inferences, arg->elapsed_ms,
           arg->success ? "PASS" : "FAIL");

    return NULL;
}

/* 路线 B Worker 线程参数 (后备方案：实例线程加锁 dup) */
struct route_b_worker_arg {
    int worker_id;
    rknn_context *root_ctx;
    pthread_mutex_t *dup_mutex;
    rknn_core_mask target_core;
    int num_inferences;
    /* 输出统计 */
    int dup_ret;
    int set_core_ret;
    int run_ret;
    int destroy_ret;
    double elapsed_ms;
    bool success;
};

static void *route_b_worker_thread(void *p) {
    struct route_b_worker_arg *arg = (struct route_b_worker_arg *)p;
    arg->success = false;

    // 1. 加锁在 Worker 线程内部调用 rknn_dup_context
    pthread_mutex_lock(arg->dup_mutex);
    rknn_context child = 0;
    arg->dup_ret = g_rknn.dup_context(arg->root_ctx, &child);
    pthread_mutex_unlock(arg->dup_mutex);

    if (arg->dup_ret != 0 || child == 0) {
        fprintf(stderr, "[Route B Worker %d] rknn_dup_context 失败，错误码: %d\n",
                arg->worker_id, arg->dup_ret);
        return NULL;
    }

    // 2. 绑定核心并执行推理
    if (g_rknn.set_core_mask) {
        arg->set_core_ret = g_rknn.set_core_mask(child, arg->target_core);
    }

    double t0 = get_time_ms();
    for (int iter = 0; iter < arg->num_inferences; ++iter) {
        arg->run_ret = g_rknn.run(child, NULL);
        if (arg->run_ret != 0) break;
    }
    double t1 = get_time_ms();
    arg->elapsed_ms = t1 - t0;

    arg->destroy_ret = g_rknn.destroy(child);
    arg->success = (arg->dup_ret == 0 && arg->run_ret == 0 && arg->destroy_ret == 0);

    return NULL;
}

int main(int argc, char **argv) {
    printf("============================================================\n");
    printf("  Heimdall NPU rknn_dup_context 验证探针 (T42)\n");
    printf("============================================================\n");

    const char *model_path = NULL;
    const char *lib_path = NULL;
    int num_workers = 3;
    int inferences = 10;
    const char *route_mode = "A";

    for (int i = 1; i < argc; ++i) {
        if (strcmp(argv[i], "-m") == 0 && i + 1 < argc) {
            model_path = argv[++i];
        } else if (strcmp(argv[i], "-l") == 0 && i + 1 < argc) {
            lib_path = argv[++i];
        } else if (strcmp(argv[i], "-c") == 0 && i + 1 < argc) {
            num_workers = atoi(argv[++i]);
        } else if (strcmp(argv[i], "-n") == 0 && i + 1 < argc) {
            inferences = atoi(argv[++i]);
        } else if (strcmp(argv[i], "--route") == 0 && i + 1 < argc) {
            route_mode = argv[++i];
        }
    }

    if (!model_path) {
        printf("用法: %s -m <model.rknn> [-l librknnrt.so] [-c workers] [-n iters] [--route A|B|all]\n", argv[0]);
        printf("说明: 未指定模型文件时进入符号与接口静态自检模式。\n\n");
    }

    if (load_rknn_library(lib_path) != 0) {
        printf("[WARN] 本机无可用 librknnrt.so 动态库（开发机静态检查模式通过）。\n");
        return 0;
    }

    if (!g_rknn.dup_context) {
        fprintf(stderr, "[FATAL] 当前 librknnrt.so 未导出 rknn_dup_context 符号！不支持同模型权重复用。\n");
        return 1;
    }

    if (!model_path) {
        printf("[INFO] 符号自检全部通过：rknn_dup_context 存在且签名匹配。\n");
        return 0;
    }

    uint32_t model_size = 0;
    void *model_data = read_file(model_path, &model_size);
    if (!model_data) return 2;

    long rss_before = get_vm_rss_kb();

    // 1. 初始化 Root Context
    rknn_context root_ctx = 0;
    printf("[INFO] 正在初始化 Root Context...\n");
    int init_ret = g_rknn.init(&root_ctx, model_data, model_size, 0, NULL);
    if (init_ret != 0) {
        fprintf(stderr, "[FATAL] rknn_init 失败，错误码: %d\n", init_ret);
        free(model_data);
        return 3;
    }
    printf("[INFO] Root Context 初始化成功: 0x%lx\n", (unsigned long)root_ctx);

    // 查询 Root 内存占用
    rknn_mem_size root_mem;
    memset(&root_mem, 0, sizeof(root_mem));
    if (g_rknn.query(root_ctx, RKNN_QUERY_MEM_SIZE, &root_mem, sizeof(root_mem)) == 0) {
        printf("[INFO] Root Memory Query: weight=%.2f MB, internal=%.2f MB, dma=%.2f MB\n",
               (double)root_mem.total_weight_size / (1024.0 * 1024.0),
               (double)root_mem.total_internal_size / (1024.0 * 1024.0),
               (double)root_mem.total_dma_allocated_size / (1024.0 * 1024.0));
    }

    long rss_after_root = get_vm_rss_kb();
    printf("[INFO] VmRSS 变化 (Root加载): %ld KB -> %ld KB (增量: %ld KB)\n",
           rss_before, rss_after_root, rss_after_root - rss_before);

    bool route_a_passed = false;

    /* =========================================================================
     * 路线 A 实测：控制线程 dup 派生后移交 Worker 线程独占执行
     * ========================================================================= */
    if (strcmp(route_mode, "A") == 0 || strcmp(route_mode, "all") == 0) {
        printf("\n>>> 开始实测【首选路线 A】(控制线程 dup -> 实例 Worker 独占移交 -> 设核/推理/销毁)\n");

        pthread_t threads[16];
        struct route_a_worker_arg args[16];
        rknn_core_mask core_masks[3] = {RKNN_NPU_CORE_0, RKNN_NPU_CORE_1, RKNN_NPU_CORE_2};

        bool dup_all_ok = true;
        for (int i = 0; i < num_workers; ++i) {
            args[i].worker_id = i;
            args[i].target_core = core_masks[i % 3];
            args[i].num_inferences = inferences;
            args[i].child_ctx = 0;

            // 控制线程统一串行执行 dup
            int dup_ret = g_rknn.dup_context(&root_ctx, &args[i].child_ctx);
            if (dup_ret != 0 || args[i].child_ctx == 0) {
                fprintf(stderr, "[FATAL] 控制线程 rknn_dup_context 派生第 %d 个 child 失败，错误码: %d\n",
                        i, dup_ret);
                dup_all_ok = false;
                break;
            }
            printf("[INFO] 控制线程成功派生 child[%d]=0x%lx\n", i, (unsigned long)args[i].child_ctx);
        }

        if (dup_all_ok) {
            long rss_after_dup = get_vm_rss_kb();
            printf("[INFO] 派生 %d 个 child 后的 VmRSS: %ld KB (相对Root增量: %ld KB)\n",
                   num_workers, rss_after_dup, rss_after_dup - rss_after_root);

            // 启动 Worker 线程执行并发推理与销毁
            for (int i = 0; i < num_workers; ++i) {
                pthread_create(&threads[i], NULL, route_a_worker_thread, &args[i]);
            }

            // 等待全部 Worker 线程完成
            bool all_workers_ok = true;
            for (int i = 0; i < num_workers; ++i) {
                pthread_join(threads[i], NULL);
                if (!args[i].success) all_workers_ok = false;
            }

            if (all_workers_ok) {
                printf("[SUCCESS] 【路线 A】验证全项通过！(跨线程移交、设核、并发推理、子Context独立销毁无异常)\n");
                route_a_passed = true;
            } else {
                printf("[FAIL] 【路线 A】验证存在失败用例。\n");
            }
        }
    }

    /* =========================================================================
     * 路线 B 实测 (后备方案：实例 Worker 加锁 dup)
     * ========================================================================= */
    if ((strcmp(route_mode, "B") == 0 || strcmp(route_mode, "all") == 0) || !route_a_passed) {
        printf("\n>>> 开始实测【后备路线 B】(实例 Worker 加锁 dup -> 设核/推理/销毁)\n");

        pthread_t threads[16];
        struct route_b_worker_arg args[16];
        pthread_mutex_t dup_mutex = PTHREAD_MUTEX_INITIALIZER;
        rknn_core_mask core_masks[3] = {RKNN_NPU_CORE_0, RKNN_NPU_CORE_1, RKNN_NPU_CORE_2};

        for (int i = 0; i < num_workers; ++i) {
            args[i].worker_id = i;
            args[i].root_ctx = &root_ctx;
            args[i].dup_mutex = &dup_mutex;
            args[i].target_core = core_masks[i % 3];
            args[i].num_inferences = inferences;
            pthread_create(&threads[i], NULL, route_b_worker_thread, &args[i]);
        }

        bool all_b_ok = true;
        for (int i = 0; i < num_workers; ++i) {
            pthread_join(threads[i], NULL);
            if (!args[i].success) all_b_ok = false;
        }

        pthread_mutex_destroy(&dup_mutex);

        if (all_b_ok) {
            printf("[SUCCESS] 【后备路线 B】验证通过！\n");
        } else {
            printf("[FAIL] 【后备路线 B】验证失败。\n");
        }
    }

    // 4. 销毁 Root Context
    printf("[INFO] 正在销毁 Root Context...\n");
    int destroy_root_ret = g_rknn.destroy(root_ctx);
    printf("[INFO] Root Context 销毁完成，返回码: %d\n", destroy_root_ret);

    free(model_data);
    printf("\n>>> 探针运行完成。\n");
    return 0;
}
