//! 自动扫描调度：开机快速扫描、按周内天定时扫描、病毒库更新后自动扫描。
//!
//! 三种触发共用一套"到点了就发起一次扫描"的逻辑，区别只在**怎么判定到点**。
//! 判定都发生在 `AppCore::poll_background`（每帧调用一次）里，内部有节流，
//! 不会每帧去问系统时间。

use crate::config::AppConfig;
use crate::localtime::Timestamp;
use std::time::{Duration, Instant};

/// 开机后延迟多久发起开机扫描：等窗口/托盘就绪，也给"数据库刚载入、引擎还在
/// 预热"留出时间——开机即扫最容易撞上引擎冷启动。
const BOOT_DELAY: Duration = Duration::from_secs(5);
/// 定时判定的节流间隔。
const CHECK_INTERVAL: Duration = Duration::from_secs(10);
/// 到点后的宽限窗口（分钟）：判定被节流错开、或应用在计划时刻刚好没运行时，
/// 仍会在这个窗口内补扫一次。窗口太大则"下午开机"也会补跑早上那一档。
const GRACE_MINUTES: i32 = 5;

/// 自动扫描的触发来源（用于提示文案）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoScanKind {
    Boot,
    Scheduled,
    AfterUpdate,
}

impl AutoScanKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            AutoScanKind::Boot => "Startup scan",
            AutoScanKind::Scheduled => "Scheduled scan",
            AutoScanKind::AfterUpdate => "Scan after update",
        }
    }
}

/// 自动扫描的跨帧状态。
pub(crate) struct AutoScanState {
    started_at: Instant,
    /// 开机扫描是否还没触发（触发一次后置位，保证每次运行只扫一次）。
    boot_pending: bool,
    /// 上一次已经触发过的定时档位（日期戳 + 计划分钟），避免同一天重复触发。
    last_fired: Option<(u32, u16)>,
    /// 上次真正对表的时间（节流用）。
    last_check: Option<Instant>,
}

impl AutoScanState {
    pub(super) fn new() -> Self {
        Self {
            started_at: Instant::now(),
            boot_pending: true,
            last_fired: None,
            last_check: None,
        }
    }

    /// 开机扫描是否该触发（只判一次）。
    fn boot_due(&mut self) -> bool {
        if !self.boot_pending {
            return false;
        }
        if self.started_at.elapsed() < BOOT_DELAY {
            return false;
        }
        self.boot_pending = false;
        true
    }

    /// 定时扫描是否到点。到点时把这一档标记为"已触发"，同一个档位不会再触发。
    fn scheduled_due(&mut self, config: &AppConfig) -> bool {
        let Some(sched) = config.scheduled_scan else {
            return false;
        };
        if !sched.is_valid() {
            return false;
        }
        let now = match self.last_check {
            Some(last) if last.elapsed() < CHECK_INTERVAL => return false,
            _ => {
                self.last_check = Some(Instant::now());
                Timestamp::now()
            }
        };
        if !is_due(&sched, &now) {
            return false;
        }
        let key = (day_stamp(&now), scheduled_minute(&sched) as u16);
        if self.last_fired == Some(key) {
            return false;
        }
        self.last_fired = Some(key);
        true
    }

    /// 本帧是否该发起一次自动扫描（开机 / 定时）。返回触发来源。
    pub(crate) fn poll(&mut self, config: &AppConfig) -> Option<AutoScanKind> {
        if config.auto_scan_on_boot && self.boot_due() {
            return Some(AutoScanKind::Boot);
        }
        if self.scheduled_due(config) {
            return Some(AutoScanKind::Scheduled);
        }
        None
    }
}

/// 日期戳（yyyyMMdd），用来区分"不同一天的同一档计划"。
fn day_stamp(t: &Timestamp) -> u32 {
    u32::from(t.year) * 10_000 + u32::from(t.month) * 100 + u32::from(t.day)
}

/// 计划时刻换算成"当天第几分钟"。
fn scheduled_minute(sched: &crate::config::ScheduleConfig) -> i32 {
    i32::from(sched.hour) * 60 + i32::from(sched.minute)
}

/// 纯判定：给定时计划与当前时刻，判断"现在该不该扫"。
///
/// - 星期不在计划内 → 不扫；
/// - 早于计划时刻 → 不扫；
/// - 计划时刻之后 `GRACE_MINUTES` 分钟内 → 扫（判定被节流错开时的补扫窗口）。
fn is_due(sched: &crate::config::ScheduleConfig, now: &Timestamp) -> bool {
    if !sched.includes_day(now.weekday()) {
        return false;
    }
    let now_minute = i32::from(now.hour) * 60 + i32::from(now.minute);
    let delta = now_minute - scheduled_minute(sched);
    (0..=GRACE_MINUTES).contains(&delta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ScheduleConfig;

    fn at(year: u16, month: u16, day: u16, hour: u16, minute: u16) -> Timestamp {
        Timestamp {
            year,
            month,
            day,
            hour,
            minute,
        }
    }

    /// 2026-10-26 是周一、10-28 是周三（weekday 换算的基础事实）。
    #[test]
    fn weekday_matches_calendar() {
        assert_eq!(at(2026, 10, 26, 0, 0).weekday(), 0, "2026-10-26 是周一");
        assert_eq!(at(2026, 10, 27, 0, 0).weekday(), 1, "2026-10-27 是周二");
        assert_eq!(at(2026, 10, 28, 0, 0).weekday(), 2, "2026-10-28 是周三");
        assert_eq!(at(2026, 1, 1, 0, 0).weekday(), 3, "2026-01-01 是周四");
        assert_eq!(at(2024, 2, 29, 0, 0).weekday(), 3, "2024-02-29 是周四");
    }

    /// 定时判定：只在计划内的星期、计划时刻（±宽限窗口）触发。
    #[test]
    fn schedule_fires_only_in_window() {
        let mut sched = ScheduleConfig::default(); // 周一 09:00
        // 周一 09:00 → 触发
        assert!(is_due(&sched, &at(2026, 10, 26, 9, 0)));
        // 周一 09:03（节流错开）→ 仍在宽限窗口内
        assert!(is_due(&sched, &at(2026, 10, 26, 9, 3)));
        // 周一 09:30 → 超出宽限窗口
        assert!(!is_due(&sched, &at(2026, 10, 26, 9, 30)));
        // 周一 08:00 → 还没到点
        assert!(!is_due(&sched, &at(2026, 10, 26, 8, 0)));
        // 周三 09:00 → 星期不在计划内
        assert!(!is_due(&sched, &at(2026, 10, 28, 9, 0)));

        // 勾上周三后，周三 09:00 也触发
        sched.days_mask |= 1 << 2;
        assert!(is_due(&sched, &at(2026, 10, 28, 9, 0)));
    }

    /// 无效计划（一天都没选）不触发。
    #[test]
    fn empty_schedule_never_fires() {
        let mut sched = ScheduleConfig::default();
        sched.days_mask = 0;
        assert!(!sched.is_valid());
        assert!(!is_due(&sched, &at(2026, 10, 26, 9, 0)));
    }
}
