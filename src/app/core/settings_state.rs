//! 设置页跨帧状态。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsTab {
    QuarantineIgnore,
    General,
}

pub(crate) struct SettingsState {
    pub(crate) tab: SettingsTab,
    pub(crate) autostart_enabled: Option<bool>,
    #[allow(dead_code)]
    pub(crate) context_menu_enabled: Option<bool>,
}

impl SettingsState {
    pub(super) fn new() -> Self {
        Self {
            tab: SettingsTab::QuarantineIgnore,
            autostart_enabled: None,
            context_menu_enabled: None,
        }
    }
}
