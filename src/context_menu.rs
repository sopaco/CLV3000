//! 右键菜单"用 CLV3000 扫描"——**仅 Windows 有真实实现**（已与用户确认：macOS 没有
//! 对应的轻量系统 API，右键菜单只能靠生成 Automator Quick Action 装进
//! `~/Library/Services/`，行为不确定，这次不做）。
//!
//! Windows 实现：往 `HKEY_CURRENT_USER\Software\Classes\*\shell\...`（任意文件）和
//! `HKEY_CURRENT_USER\Software\Classes\Directory\shell\...`（文件夹）各写一个键，
//! 不需要管理员权限（`HKCU\Software\Classes` 是当前用户可写的分支，效果上跟
//! `HKEY_CLASSES_ROOT` 合并生效，是给单个应用注册右键菜单的标准做法，不用碰
//! `HKEY_LOCAL_MACHINE`）。菜单点击后调用 `"<exe>" --scan-path "%1"`（见
//! `lifecycle::parse_scan_path`），已经在跑的实例通过 `single_instance` 的转发
//! 机制接住这个请求，不会因为单实例锁被拦下。

/// 探测右键菜单当前是否已注册（文件 + 文件夹两组键都存在才算"已启用"）。
/// 只在 `app/settings.rs` 的 `#[cfg(windows)]` 分支调用，非 Windows 编译目标上
/// 这两个 facade 函数是 dead code——保留统一签名方便调用侧不用按平台分叉，
/// `#[allow(dead_code)]` 压掉这个预期内的警告。
#[allow(dead_code)]
pub fn is_enabled() -> bool {
    imp::is_enabled()
}

/// 开启/关闭右键菜单注册。非 Windows 平台直接返回错误——调用方（设置页）据此
/// 渲染一张"暂不支持"的灰态卡片。
#[allow(dead_code)]
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    imp::set_enabled(enabled)
}

#[cfg(windows)]
mod imp {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
        RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegOpenKeyExW, RegSetValueExW,
    };

    /// 菜单在 Explorer 右键里显示的文字。
    const MENU_LABEL: &str = "Scan with CLV3000";
    /// 注册表里这组键的名字（不是显示文字，是 `shell\<这个>`，随便取一个不会跟别的
    /// 软件撞名的标识符）。
    const VERB_KEY: &str = "CLV3000Scan";

    /// 两个挂载点：`*`（任意文件类型）与 `Directory`（文件夹）。
    const ROOTS: &[&str] = &["*", "Directory"];

    fn exe_path() -> Result<String, String> {
        std::env::current_exe()
            .map(|p| p.display().to_string())
            .map_err(|e| format!("Failed to resolve executable path: {e}"))
    }

    pub fn is_enabled() -> bool {
        ROOTS.iter().all(|root| verb_key_exists(root))
    }

    fn verb_key_exists(root: &str) -> bool {
        let subkey = HSTRING::from(format!("Software\\Classes\\{root}\\shell\\{VERB_KEY}"));
        let mut hkey = HKEY::default();
        // SAFETY: 标准 Win32 注册表只读探测调用。
        let status = unsafe {
            RegOpenKeyExW(HKEY_CURRENT_USER, &subkey, Some(0), KEY_QUERY_VALUE, &mut hkey)
        };
        if status == ERROR_SUCCESS {
            unsafe {
                let _ = RegCloseKey(hkey);
            }
            true
        } else {
            false
        }
    }

    pub fn set_enabled(enabled: bool) -> Result<(), String> {
        if enabled {
            let exe = exe_path()?;
            for root in ROOTS {
                register_root(root, &exe)?;
            }
        } else {
            for root in ROOTS {
                unregister_root(root);
            }
        }
        Ok(())
    }

    /// 写一组键：`<root>\shell\CLV3000Scan`（默认值=菜单文字，`Icon`=exe 路径）+
    /// 子键 `command`（默认值 = `"<exe>" --scan-path "%1"`）。
    fn register_root(root: &str, exe: &str) -> Result<(), String> {
        let verb_subkey = HSTRING::from(format!("Software\\Classes\\{root}\\shell\\{VERB_KEY}"));
        let verb_hkey = create_key(&verb_subkey)?;
        set_default_value(verb_hkey, MENU_LABEL)?;
        set_named_value(verb_hkey, "Icon", exe)?;
        unsafe {
            let _ = RegCloseKey(verb_hkey);
        }

        let command_subkey =
            HSTRING::from(format!("Software\\Classes\\{root}\\shell\\{VERB_KEY}\\command"));
        let command_hkey = create_key(&command_subkey)?;
        let command = format!("\"{exe}\" --scan-path \"%1\"");
        set_default_value(command_hkey, &command)?;
        unsafe {
            let _ = RegCloseKey(command_hkey);
        }
        Ok(())
    }

    /// 递归删除 `<root>\shell\CLV3000Scan`（连同其 `command` 子键一起删掉）。
    /// 键本来就不存在时 `RegDeleteTreeW` 会返回错误，忽略——效果上"关闭"已经达成。
    fn unregister_root(root: &str) {
        let verb_subkey = HSTRING::from(format!("Software\\Classes\\{root}\\shell\\{VERB_KEY}"));
        // SAFETY: 标准 Win32 调用，`HKEY_CURRENT_USER` 是预定义句柄不需要关闭。
        unsafe {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &verb_subkey);
        }
    }

    fn create_key(subkey: &HSTRING) -> Result<HKEY, String> {
        use windows::Win32::System::Registry::KEY_SET_VALUE;
        use windows::core::PCWSTR;

        let mut hkey = HKEY::default();
        // SAFETY: 参数均为本函数内构造的有效句柄/字符串，无并发访问。
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                subkey,
                Some(0),
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut hkey,
                None,
            )
        };
        if status == ERROR_SUCCESS {
            Ok(hkey)
        } else {
            Err(format!("Failed to create registry key (error {})", status.0))
        }
    }

    /// 写键的"默认值"（`(Default)`，`Explorer` 右键菜单读的就是这个）。
    fn set_default_value(hkey: HKEY, value: &str) -> Result<(), String> {
        write_string_value(hkey, None, value)
    }

    fn set_named_value(hkey: HKEY, name: &str, value: &str) -> Result<(), String> {
        write_string_value(hkey, Some(name), value)
    }

    fn write_string_value(hkey: HKEY, name: Option<&str>, value: &str) -> Result<(), String> {
        use windows::core::PCWSTR;

        let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes: &[u8] =
            unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) };
        let name_hstring = name.map(HSTRING::from);
        // `PCWSTR(ptr)` 直接构造，不走 `.into()`——`RegSetValueExW` 的 `lpValueName`
        // 是个泛型 `Param<PCWSTR>`，裸指针 `.into()` 没有唯一目标类型可推导。
        let name_ptr = PCWSTR(
            name_hstring
                .as_ref()
                .map(|h| h.as_ptr())
                .unwrap_or(std::ptr::null()),
        );
        // SAFETY: `bytes`/`name_ptr` 生命周期覆盖这次调用；`name` 为 `None` 时写默认值，
        // 符合 `RegSetValueExW` 对 `lpValueName=NULL` 的约定。
        let status = unsafe { RegSetValueExW(hkey, name_ptr, Some(0), REG_SZ, Some(bytes)) };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("Failed to write registry value (error {})", status.0))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn is_enabled() -> bool {
        false
    }

    pub fn set_enabled(_enabled: bool) -> Result<(), String> {
        Err("Not supported on this platform".to_string())
    }
}
