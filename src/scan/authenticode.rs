//! 可信签名预筛：在把文件交给 ClamAV 之前，先用系统级 API 校验数字签名，
//! 通过校验（链到受信根、未篡改）的文件直接判为干净并跳过 ClamAV。
//!
//! 为什么有用：
//! - 闪电/全盘扫描里绝大多数可执行文件要么是系统/厂商签名文件，要么是知名厂商
//!   （浏览器、驱动、运行库）的嵌入签名文件。这一道直接砍掉首扫里 60~90% 的工作量——
//!   而且**第一次扫描就生效**，不像基因缓存要第二次才见效。
//! - 校验是系统级 API，单文件毫秒级，远低于 ClamAV 关掉 bytecode 后仍有 100~300ms。
//!
//! 安全权衡（重要）：
//! - 这是**启发式加速**而非安全保证。被滥用/被盗证书签名的恶意文件、以及"自带合法
//!   签名但行为恶意"的文件，会被放过。基因缓存（绑定病毒库版本）+ 用户偶尔跑一次
//!   全盘扫描作为兜底。
//! - 默认开启；把 `ENABLE` 改成 `false` 即整体关闭（等价退回"只靠基因缓存"）。
//!
//! 平台实现：
//! - Windows：用 `WinVerifyTrust` 校验 PE 嵌入签名 / catalog 签名（见 `win` 子模块）。
//! - macOS：用 `codesign --verify` 校验 Mach-O 的代码签名（见 `macos` 子模块）。
//! - 其它目标（Linux 等）不编译本模块——mock 引擎不引用它。

#![cfg(any(windows, target_os = "macos"))]

use std::path::Path;

/// 总开关。改成 `false` 即关闭可信签名预筛（退回只靠基因缓存）。
pub const ENABLE: bool = true;

/// 判断文件是否"可信签名"。命中以下任一即返回 true：
/// 1. 是 PE/Mach-O 可执行文件；
/// 2. 其系统级代码签名校验通过。
///
/// 任一检查需要打开文件、解析签名，失败或无签名都返回 false（交给 ClamAV 处理）。
pub fn is_trusted_signed(path: &Path) -> bool {
    if !ENABLE {
        return false;
    }
    #[cfg(windows)]
    {
        win::is_pe_file(path) && win::verify(path)
    }
    #[cfg(target_os = "macos")]
    {
        macos::is_macho_file(path) && macos::verify(path)
    }
}

#[cfg(windows)]
mod win {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Security::WinTrust::{
        WinVerifyTrust, DRIVER_ACTION_VERIFY, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA,
        WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
        WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    };

    /// 这些后缀才可能是 PE 可执行文件，只对它们跑签名校验，避免对文档/图片等
    /// 无意义地调用 WinVerifyTrust 浪费时间。
    pub fn is_pe_file(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| {
                matches!(
                    e.to_ascii_lowercase().as_str(),
                    "exe" | "dll" | "sys" | "ocx" | "scr" | "cpl" | "mui" | "ax" | "drv" | "efi"
                )
            })
            .unwrap_or(false)
    }

    /// 判断文件是否"可信签名"。命中以下任一即返回 true：
    /// 1. 嵌入签名（第三方安装包、驱动、运行库等），链到受信根；
    /// 2. 目录签名 / catalog（Windows 系统文件大多无嵌入签名，而是哈希登记在某个
    ///    catalog 里，由 `DRIVER_ACTION_VERIFY` 校验）。
    pub fn verify(path: &Path) -> bool {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // 先查嵌入签名（覆盖绝大多数第三方 PE）。
        if unsafe { verify_trusted(&wide, &WINTRUST_ACTION_GENERIC_VERIFY_V2) } {
            return true;
        }
        // 再查目录签名（覆盖 System32 / WinSxS 里的系统文件）。
        if unsafe { verify_trusted(&wide, &DRIVER_ACTION_VERIFY) } {
            return true;
        }
        false
    }

    /// 对给定 action GUID 跑一次 WinVerifyTrust（VERIFY → CLOSE 两段式），成功（S_OK）返回 true。
    ///
    /// 不做吊销检查（速度优先）：`fdwRevocationChecks = WTD_REVOKE_NONE`。
    /// `WINTRUST_DATA` / `WINTRUST_FILE_INFO` 都是 POD，用 zeroed 初始化后只填用到的字段，
    /// 没填的指针/标志保持 NULL/0，符合 WinTrust 的预期。
    #[allow(unused_assignments)] // 第二次调用经裸指针读取 dwStateAction，静态分析看不到
    unsafe fn verify_trusted(wide_path: &[u16], action: &windows::core::GUID) -> bool {
        let mut file_info: WINTRUST_FILE_INFO = unsafe { std::mem::zeroed() };
        file_info.cbStruct = std::mem::size_of::<WINTRUST_FILE_INFO>() as u32;
        file_info.pcwszFilePath = PCWSTR(wide_path.as_ptr());
        // hFile 留 NULL：让 WinTrust 自己按路径打开文件（更稳，避免我们持有的句柄权限问题）。

        let mut data: WINTRUST_DATA = unsafe { std::mem::zeroed() };
        data.cbStruct = std::mem::size_of::<WINTRUST_DATA>() as u32;
        data.dwUIChoice = WTD_UI_NONE; // 不弹任何系统对话框
        data.fdwRevocationChecks = WTD_REVOKE_NONE;
        data.dwUnionChoice = WTD_CHOICE_FILE;
        data.Anonymous.pFile = &mut file_info;
        data.dwStateAction = WTD_STATEACTION_VERIFY;

        let hwnd = HWND::default();
        // WinVerifyTrust 要的是 `*mut GUID` 与 `*mut c_void`，且返回 i32（HRESULT，S_OK==0）。
        let action_ptr = action as *const windows::core::GUID as *mut windows::core::GUID;
        let data_ptr = &mut data as *mut WINTRUST_DATA;
        let verify = unsafe { WinVerifyTrust(hwnd, action_ptr, data_ptr as *mut std::ffi::c_void) };

        // 必须发 CLOSE 把 WinTrust 内部状态句柄释放掉，否则会内存泄漏。
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        let _ = unsafe { WinVerifyTrust(hwnd, action_ptr, data_ptr as *mut std::ffi::c_void) };

        verify == 0
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::Path;
    use std::process::Command;

    /// 判断是否是 Mach-O 可执行文件（含 fat/universal 二进制）。命中才跑 `codesign`
    /// 校验，避免对文档/图片无意义地 spawn 子进程。实现见 `scan::is_macho_file`——
    /// 与 `full_scan::real_macos::walk` 共用同一份魔数判断，不重复维护两份。
    pub fn is_macho_file(path: &Path) -> bool {
        crate::scan::is_macho_file(path)
    }

    /// 用系统 `codesign --verify` 校验 Mach-O 的代码签名：签名且未被篡改 → 退出码 0 → 可信。
    /// 未签名 / 签名无效 / 无法读取则返回 false，交给 ClamAV 处理。
    ///
    /// 注意：与 Windows 一样是**启发式加速**，不做 Gatekeeper/公证的联网复核，
    /// 也不做吊销检查——速度优先，安全兜底交给基因缓存与全盘扫描。
    pub fn verify(path: &Path) -> bool {
        Command::new("codesign")
            .arg("--verify")
            .arg("--verbose=1")
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}
