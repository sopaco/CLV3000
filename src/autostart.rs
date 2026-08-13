//! 开机自启动：设置页「Autostart」tab 的开关。启动参数复用已有的
//! `--tray-only`（见 `lifecycle::parse_start_tray_only`）——自启动是为了"电脑一开
//! 机就默默常驻保护"，不应该每次都弹出主窗口打扰用户。
//!
//! - Windows：`HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`，
//!   不需要管理员权限（写的是当前用户的 Run 键，不是 `HKEY_LOCAL_MACHINE`）。
//! - macOS：`~/Library/LaunchAgents\<label>.plist` + `launchctl load/unload`。
//! - 其它平台（开发预览）：内存里一个 `AtomicBool`，仅供设置页点得动。

/// 探测当前是否已注册为开机自启动。
pub fn is_enabled() -> bool {
    imp::is_enabled()
}

/// 开启/关闭开机自启动。失败时返回可展示给用户的错误信息。
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    imp::set_enabled(enabled)
}

/// 拿当前 exe 的绝对路径；拿不到就没法写自启动项，直接报错而不是写一个空路径。
fn exe_path() -> Result<std::path::PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("Failed to resolve executable path: {e}"))
}

#[cfg(windows)]
mod imp {
    use super::exe_path;
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    };

    const SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const VALUE_NAME: &str = "CLV3000";

    /// 期望写进 Run 键的值：`"<exe路径>" --tray-only`。带引号防止路径里有空格
    /// （安装目录里含空格是常见场景）被系统解析成多个参数。
    fn expected_command(exe: &std::path::Path) -> String {
        format!("\"{}\" --tray-only", exe.display())
    }

    pub fn is_enabled() -> bool {
        let Ok(exe) = exe_path() else { return false };
        let Some(current) = read_value() else {
            return false;
        };
        current == expected_command(&exe)
    }

    pub fn set_enabled(enabled: bool) -> Result<(), String> {
        let exe = exe_path()?;
        if enabled {
            write_value(&expected_command(&exe))
        } else {
            delete_value()
        }
    }

    /// 打开（不存在就不管，`RegOpenKeyExW` 本身不会创建）Run 键；`Run` 键在所有
    /// 现代 Windows 上都随系统预置，不存在的概率极低，读操作用 `RegOpenKeyExW`
    /// 就够，写操作见下面 `write_value` 用 `RegCreateKeyExW` 兜底"键不存在"的极端情况。
    fn open_key(write: bool) -> Option<HKEY> {
        let subkey = HSTRING::from(SUBKEY);
        let access = if write { KEY_SET_VALUE } else { KEY_QUERY_VALUE };
        let mut hkey = HKEY::default();
        // SAFETY: 都是标准 Win32 注册表调用，参数是本函数内构造的有效句柄/字符串。
        let status =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &subkey, Some(0), access, &mut hkey) };
        if status == ERROR_SUCCESS {
            Some(hkey)
        } else {
            None
        }
    }

    fn read_value() -> Option<String> {
        let hkey = open_key(false)?;
        let value_name = HSTRING::from(VALUE_NAME);
        let mut buf = [0u16; 1024];
        let mut cb_data = (buf.len() * 2) as u32;
        // SAFETY: `buf` 缓冲区大小与 `cb_data` 一致，`lptype` 传 None 表示不关心类型。
        let status = unsafe {
            RegQueryValueExW(
                hkey,
                &value_name,
                None,
                None,
                Some(buf.as_mut_ptr() as *mut u8),
                Some(&mut cb_data),
            )
        };
        unsafe {
            let _ = RegCloseKey(hkey);
        }
        if status != ERROR_SUCCESS {
            return None;
        }
        // `cb_data` 是字节数，Run 值通常是 REG_SZ（UTF-16 + 结尾 NUL）。
        let len_u16 = (cb_data as usize / 2).min(buf.len());
        let s = String::from_utf16_lossy(&buf[..len_u16]);
        Some(s.trim_end_matches('\0').to_string())
    }

    fn write_value(command: &str) -> Result<(), String> {
        use windows::Win32::System::Registry::RegCreateKeyExW;

        let subkey = HSTRING::from(SUBKEY);
        let mut hkey = HKEY::default();
        // `RegCreateKeyExW`：键已存在则等价于打开，不存在才真的创建——Run 键几乎
        // 总是已存在，这里只是防御性兜底。
        // SAFETY: 参数均为本函数内构造的有效句柄/字符串，无并发访问。
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                &subkey,
                Some(0),
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut hkey,
                None,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(format!("Failed to open registry key (error {})", status.0));
        }

        let value_name = HSTRING::from(VALUE_NAME);
        // REG_SZ 要求数据以 UTF-16 NUL 结尾。
        let wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes: &[u8] =
            unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) };
        // SAFETY: `bytes` 生命周期覆盖这次调用，`hkey` 刚打开有效。
        let status =
            unsafe { RegSetValueExW(hkey, &value_name, Some(0), REG_SZ, Some(bytes)) };
        unsafe {
            let _ = RegCloseKey(hkey);
        }
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("Failed to write registry value (error {})", status.0))
        }
    }

    fn delete_value() -> Result<(), String> {
        let Some(hkey) = open_key(true) else {
            // 键都打不开：大概率本来就没有这个自启动项，视为"关闭"已经成功。
            return Ok(());
        };
        let value_name = HSTRING::from(VALUE_NAME);
        // SAFETY: 标准 Win32 调用。
        let status = unsafe { RegDeleteValueW(hkey, &value_name) };
        unsafe {
            let _ = RegCloseKey(hkey);
        }
        use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!("Failed to remove registry value (error {})", status.0))
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::exe_path;
    use std::path::PathBuf;

    const LABEL: &str = "com.sgnetworks.clv3000";

    fn plist_path() -> Option<PathBuf> {
        let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
        Some(home.join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
    }

    pub fn is_enabled() -> bool {
        plist_path().map(|p| p.is_file()).unwrap_or(false)
    }

    pub fn set_enabled(enabled: bool) -> Result<(), String> {
        let Some(path) = plist_path() else {
            return Err("Failed to resolve home directory".to_string());
        };
        if enabled {
            let exe = exe_path()?;
            if let Some(parent) = path.parent() {
                crate::paths::ensure_dir(parent);
            }
            let plist = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>--tray-only</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
                exe = exe.display()
            );
            // 只写文件，不调 `launchctl load`——`load` 会让 launchd 立刻按
            // `RunAtLoad` 把它跑起来，等于用户刚勾选"开机自启动"这一下就在当前
            // 会话里多开了一个进程，撞上单实例锁弹"已经在运行"（这正是这个函数
            // 曾经的行为，是个 bug，不是预期）。勾选这个开关的意图只是"设置好
            // 下次登录自动启动"，不是"现在立刻启动一次"。macOS 登录时会自动扫
            // `~/Library/LaunchAgents/` 下所有 plist 并加载，不需要现在手动
            // `load` 才能在下次登录生效。
            std::fs::write(&path, plist)
                .map_err(|e| format!("Failed to write launch agent: {e}"))?;
            Ok(())
        } else {
            // 同理不调 `launchctl unload`——如果这个 LaunchAgent 是上次登录时被
            // launchd 加载起来的（也就是当前正在跑的这个进程本身），`unload` 会
            // 把它直接杀掉，而"关闭开机自启动"应该只影响以后的登录，不该干掉
            // 用户正在用的当前会话。只删文件：下次登录 launchd 就不会再加载它，
            // 当前这次运行不受影响。
            if path.exists() {
                std::fs::remove_file(&path)
                    .map_err(|e| format!("Failed to remove launch agent: {e}"))?;
            }
            Ok(())
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};

    static MOCK_ENABLED: AtomicBool = AtomicBool::new(false);

    pub fn is_enabled() -> bool {
        MOCK_ENABLED.load(Ordering::Relaxed)
    }

    pub fn set_enabled(enabled: bool) -> Result<(), String> {
        MOCK_ENABLED.store(enabled, Ordering::Relaxed);
        Ok(())
    }
}
