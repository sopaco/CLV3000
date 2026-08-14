//! 四个业务页面的渲染入口。

mod dashboard;
mod scan;
mod virus_db;

pub(in crate::app) use dashboard::dashboard_page;
pub(in crate::app) use scan::{full_scan_page, quick_scan_page};
pub(in crate::app) use virus_db::virus_db_page;
