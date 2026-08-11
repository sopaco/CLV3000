//! 资源监控：独立后台线程，每秒采集一次 CPU/内存占用，通过 channel 发给 UI。
//! 只做采集和展示，不做任何告警逻辑。

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
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
    stop: Arc<(Mutex<bool>, Condvar)>,
}

impl Drop for SysMonHandle {
    fn drop(&mut self) {
        // 置位停止标记并唤醒后台线程，让它立刻从 park_timeout 返回并退出，
        // 不必空转等到下一次采样周期结束。
        let (lock, cvar) = &*self.stop;
        *lock.lock().unwrap() = true;
        cvar.notify_one();
    }
}

/// 启动资源监控后台线程，返回一个可以轮询最新采样值的 handle。
/// `ctx` 用于每产出一份采样就 `request_repaint` 唤醒 UI——这样底部资源条按 1Hz
/// 刷新完全由数据驱动，UI 线程不需要为它为维持任何定时重绘心跳。
pub fn spawn(ctx: egui::Context) -> SysMonHandle {
    let (tx, rx): (Sender<ResourceSample>, Receiver<ResourceSample>) = std::sync::mpsc::channel();
    let stop = Arc::new((Mutex::new(false), Condvar::new()));
    let stop_flag = Arc::clone(&stop);

    std::thread::spawn(move || {
        let mut sys = System::new();
        // sysinfo 要求两次 refresh_cpu_usage 之间至少间隔 MINIMUM_CPU_UPDATE_INTERVAL
        // 才能算出有意义的百分比，第一次先热身一下。
        sys.refresh_cpu_usage();
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);

        // 系统级 CPU/内存本来就是全机器汇总的数字，其它进程一动就会跳一下，是正常
        // 现象，不是这个程序占用有问题；但每秒原始值直接怼到 UI 上看着确实很闹，
        // 这里用一个简单的指数滑动平均把曲线捋顺一点，牺牲一点实时性换取不那么跳。
        const SMOOTHING: f32 = 0.3; // 越小越平滑（但跟手慢），越大越贴近原始读数
        let mut smoothed_cpu: Option<f32> = None;
        let mut smoothed_mem: Option<f64> = None;

        loop {
            // 等待停止标记或 1 秒到期——用条件变量 + park_timeout 替代原先的
            // 10×100ms 自旋轮询：平时线程真正睡死、零 CPU；关闭时由 Drop 的
            // notify_one 立刻唤醒，几乎无延迟退出。
            {
                let (lock, cvar) = &*stop_flag;
                let guard = lock.lock().unwrap();
                if *guard {
                    break;
                }
                let (guard, _timeout) = cvar
                    .wait_timeout(guard, Duration::from_millis(1_000))
                    .unwrap();
                if *guard {
                    break;
                }
                drop(guard);
            }

            sys.refresh_cpu_usage();
            sys.refresh_memory();

            let raw_cpu = sys.global_cpu_usage();
            let cpu = match smoothed_cpu {
                Some(prev) => prev + (raw_cpu - prev) * SMOOTHING,
                None => raw_cpu,
            };
            smoothed_cpu = Some(cpu);

            let raw_mem = sys.used_memory() as f64;
            let mem = match smoothed_mem {
                Some(prev) => prev + (raw_mem - prev) * SMOOTHING as f64,
                None => raw_mem,
            };
            smoothed_mem = Some(mem);

            let sample = ResourceSample {
                cpu_percent: cpu,
                mem_used_bytes: mem.round() as u64,
                mem_total_bytes: sys.total_memory(),
            };
            if tx.send(sample).is_err() {
                break;
            }
            // 唤醒 UI 消费这份采样并刷新资源条（约 1Hz）。
            ctx.request_repaint();
        }
    });

    SysMonHandle { rx, stop }
}
