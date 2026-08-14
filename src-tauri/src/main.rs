// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::OnceLock;

/// full-product / builtin-grok 在 debug 下会生成很大的异步状态机；Tokio 默认 worker
/// 栈约 2MiB，自动驾驶/内置执行路径上曾出现 `tokio-rt-worker` stack overflow。
/// 必须在任何 `tauri::async_runtime` 使用之前安装更大栈的 runtime。
fn install_async_runtime() {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let runtime = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("metheus-tokio")
            .thread_stack_size(16 * 1024 * 1024)
            .build()
            .expect("failed to create Tokio runtime with enlarged worker stack")
    });
    tauri::async_runtime::set(runtime.handle().clone());
}

fn main() {
    install_async_runtime();
    metheus_lib::run()
}
