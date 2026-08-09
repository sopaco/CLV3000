//! 极简本地时间工具：Windows 下只依赖 `windows` crate 的 `GetLocalTime`，不引入 chrono
//! 之类的重量级依赖。非 Windows（macOS/Linux 开发机预览用）没有这个 API，改用
//! `SystemTime` + 一个不依赖任何 crate 的公历换算算法，注意这条路径算出来的是 **UTC**
//! 时间（没有查时区表），只用来在预览时让"今天/X月X日"这类文案有真实数据可看，
//! 不追求跟真实本地时间分秒不差。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Timestamp {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
}

impl Timestamp {
    /// 取当前时间。
    pub fn now() -> Self {
        imp::now()
    }

    /// 渲染成"今天 09:12" / "8月8日 09:12" 这种展示文案，`today` 用于判断是否是当天。
    pub fn display_relative_to(&self, today: &Timestamp) -> String {
        if self.year == today.year && self.month == today.month && self.day == today.day {
            format!("Today {:02}:{:02}", self.hour, self.minute)
        } else {
            format!(
                "{:02}/{:02} {:02}:{:02}",
                self.month, self.day, self.hour, self.minute
            )
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::Timestamp;
    use windows::Win32::System::SystemInformation::GetLocalTime;

    pub fn now() -> Timestamp {
        // SAFETY: GetLocalTime 只是把当前本地时间写进一个 SYSTEMTIME 输出参数，无其它前置条件。
        let st = unsafe { GetLocalTime() };
        Timestamp {
            year: st.wYear,
            month: st.wMonth,
            day: st.wDay,
            hour: st.wHour,
            minute: st.wMinute,
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Timestamp;
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn now() -> Timestamp {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0) as i64;
        let days = secs.div_euclid(86_400);
        let secs_of_day = secs.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        Timestamp {
            year: year as u16,
            month: month as u16,
            day: day as u16,
            hour: (secs_of_day / 3600) as u16,
            minute: ((secs_of_day % 3600) / 60) as u16,
        }
    }

    /// Howard Hinnant 的公历日期换算算法（days-since-1970-01-01 -> 年/月/日），
    /// 公开算法，不依赖任何日期库：http://howardhinnant.github.io/date_algorithms.html
    fn civil_from_days(z: i64) -> (i64, u32, u32) {
        let z = z + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097); // [0, 146096]
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
        let mp = (5 * doy + 2) / 153; // [0, 11]
        let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
        let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
        (y + i64::from(m <= 2), m as u32, d as u32)
    }
}
