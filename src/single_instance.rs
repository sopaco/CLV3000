//! 单实例保护：保证同时只有一个 CLV3000 在跑。
//!
//! 平台实现：
//! - Windows：具名 Mutex（`Global\CLV3000_SingleInstance_Mutex`），拿到的 HANDLE 故意不
//!   close，让它随进程退出自动释放。
//! - macOS（及其它 Unix）：在 `app_data_dir` 下绑定一个 Unix 域 socket
//!   `clv3000.sock`。绑定成功即拿到锁；若文件已存在，先尝试连一下——连得上说明
//!   上一个实例还活着（本实例应退出），连不上说明是崩溃残留的僵尸 socket，删掉重绑。
//! - 其它（Linux 等开发机预览用）：直接放行，不做单实例限制——反正只是拿来看 UI。

#[cfg(windows)]
pub use real::acquire;
#[cfg(target_os = "macos")]
pub use macos::acquire;
#[cfg(not(any(windows, target_os = "macos")))]
pub use mock::acquire;

#[cfg(windows)]
mod real {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;

    const MUTEX_NAME: &str = "Global\\CLV3000_SingleInstance_Mutex";

    /// 如果已经有一个实例在跑，返回 `false`（调用者应该直接退出并提示用户）。
    pub fn acquire() -> bool {
        // 开发调试用：设置 `CLV3000_ALLOW_MULTIPLE_INSTANCES` 可绕过单实例锁。
        if std::env::var_os("CLV3000_ALLOW_MULTIPLE_INSTANCES").is_some() {
            return true;
        }
        let name = HSTRING::from(MUTEX_NAME);
        // SAFETY: 不传自定义安全属性，名字是一个 'static 常量字符串对应的 HSTRING。
        let result = unsafe { CreateMutexW(None, true, &name) };
        match result {
            Ok(_handle) => {
                // 即使创建成功，也可能是"打开了一个已存在的同名 Mutex"——用 GetLastError 区分。
                let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
                !already_exists
            }
            Err(_) => {
                // 创建失败也别把程序卡死，允许继续运行（极少数情况，比如权限异常）。
                true
            }
        }
    }
}

/// macOS / Unix：用 Unix 域 socket 充当文件锁。绑定成功 = 拿到锁。
#[cfg(target_os = "macos")]
mod macos {
    use std::os::unix::net::{UnixListener, UnixStream};

    const SOCK_NAME: &str = "clv3000.sock";

    /// 如果已经有一个实例在跑，返回 `false`（调用者应该直接退出并提示用户）。
    pub fn acquire() -> bool {
        // 开发调试用：设置 `CLV3000_ALLOW_MULTIPLE_INSTANCES` 可绕过单实例锁，
        // 强制启一个新实例（方便对比/测试新构建，而不必先揪出旧实例）。
        if std::env::var_os("CLV3000_ALLOW_MULTIPLE_INSTANCES").is_some() {
            return true;
        }
        let dir = crate::paths::app_data_dir();
        let _ = std::fs::create_dir_all(&dir);
        let sock = dir.join(SOCK_NAME);

        match UnixListener::bind(&sock) {
            Ok(listener) => {
                // 持有 listener 到进程退出（Drop 释放 socket 文件）。用 forget 让它活到进程结束。
                std::mem::forget(listener);
                true
            }
            Err(_) => {
                // socket 文件已存在：连一下判断上一个实例是否还活着。
                if UnixStream::connect(&sock).is_ok() {
                    return false; // 上一个实例还活着 → 本实例退出
                }
                // 连不上 = 僵尸 socket（崩溃残留）→ 删掉重绑一次。
                let _ = std::fs::remove_file(&sock);
                match UnixListener::bind(&sock) {
                    Ok(listener) => {
                        std::mem::forget(listener);
                        true
                    }
                    Err(_) => false,
                }
            }
        }
    }
}

/// 第二个实例启动时调用：弹一个原生提示，告诉用户"已经在运行、请先退出旧实例"，
/// 避免静默退出让人误以为 `cargo run` 没生效、反复点到旧（可能有 bug 的）实例上。
#[cfg(target_os = "macos")]
pub fn notice_already_running() {
    let script = "display alert \"CLV3000 已经在运行\" message \
        \"CLV3000 的一个实例已在运行（通常在菜单栏托盘中）。请先在托盘菜单退出旧实例，\
        再重新启动以加载最新版本。\" as warning";
    let _ = std::process::Command::new("osascript")
        .args(["-e", script])
        .status();
}

#[cfg(windows)]
pub fn notice_already_running() {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONWARNING, MB_OK};

    let text: Vec<u16> = "CLV3000 已经在运行。请先退出旧实例，再重新启动以加载最新版本。"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let caption: Vec<u16> = "CLV3000"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_OK | MB_ICONWARNING,
        );
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn notice_already_running() {
    eprintln!("CLV3000 is already running.");
}

#[cfg(not(any(windows, target_os = "macos")))]
mod mock {
    pub fn acquire() -> bool {
        true
    }
}
