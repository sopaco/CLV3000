//! 资源监控：独立后台线程，每秒采集一次 CPU/内存占用，通过 channel 发给 UI。
//! 只做采集和展示，不做任何告警逻辑。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;
use sysinfo::System;

#[derive(Debug, Clone, Copy, Default)]
pub struct ResourceSample {
    pub cpu_percent: f32,
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
}

impl ResourceSample {
    pub fn mem_percent(&self) -> f32 {
        if self.mem_total_bytes == 0 {
            0.0
        } else {
            (self.mem_used_bytes as f64 / self.mem_total_bytes as f64 * 100.0) as f32
        }
    }
}

pub struct SysMonHandle {
    pub rx: Receiver<ResourceSample>,
    stop: Arc<AtomicBool>,
}

impl Drop for SysMonHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// 启动资源监控后台线程，返回一个可以轮询最新采样值的 handle。
pub fn spawn() -> SysMonHandle {
    let (tx, rx): (Sender<ResourceSample>, Receiver<ResourceSample>) = std::sync::mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);

    std::thread::spawn(move || {
        let mut sys = System::new_all();
        // sysinfo 要求两次 refresh_cpu_usage 之间至少间隔 MINIMUM_CPU_UPDATE_INTERVAL
        // 才能算出有意义的百分比，第一次先热身一下。
        sys.refresh_cpu_usage();
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);

        while !stop_flag.load(Ordering::SeqCst) {
            sys.refresh_cpu_usage();
            sys.refresh_memory();
            let sample = ResourceSample {
                cpu_percent: sys.global_cpu_usage(),
                mem_used_bytes: sys.used_memory(),
                mem_total_bytes: sys.total_memory(),
            };
            if tx.send(sample).is_err() {
                break;
            }
            // 用短睡眠间隔轮询停止标记，这样关闭程序时不用等一整秒。
            for _ in 0..10 {
                if stop_flag.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });

    SysMonHandle { rx, stop }
}
