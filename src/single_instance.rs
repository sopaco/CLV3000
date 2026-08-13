//! 单实例保护：保证同时只有一个 CLV3000 在跑。
//!
//! 平台实现：
//! - Windows：具名 Mutex（`Global\CLV3000_SingleInstance_Mutex`），拿到的 HANDLE 故意不
//!   close，让它随进程退出自动释放。
//! - macOS（及其它 Unix）：在 `app_data_dir` 下绑定一个 Unix 域 socket
//!   `clv3000.sock`。绑定成功即拿到锁；若文件已存在，先尝试连一下——连得上说明
//!   上一个实例还活着（本实例应退出），连不上说明是崩溃残留的僵尸 socket，删掉重绑。
//! - 其它（Linux 等开发机预览用）：直接放行，不做单实例限制——反正只是拿来看 UI。
//!
//! ## 扫描请求转发（`forward_scan_request` / `start_scan_request_listener`）
//!
//! 右键菜单"用 CLV3000 扫描"/`--scan-path` 冷启动时，如果已经有一个实例在跑，
//! 不能像过去那样简单弹"已经在运行"就退出——那样右键菜单等于什么也没做。第二个
//! 进程改成把扫描路径转发给已经在跑的那个实例（`forward_scan_request`），主实例
//! 起一个后台监听（`start_scan_request_listener`，须在 `main` 里、`wakeup::init()`
//! 之后尽早调用——不依赖 eframe 是否已启动，Windows tray-only 下可能长期停在
//! `main::wait_in_tray` 里，此时也必须能收到转发请求）把收到的路径推进
//! `wakeup::push_scan_request`，跟托盘/菜单事件走同一套"全局队列 + ping()"唤醒
//! 机制，`wait_in_tray`（eframe 未启动）和 `App::logic`（eframe 已启动）都从那个
//! 队列排空，不用关心当前处于哪个阶段。

use std::path::Path;

#[cfg(windows)]
pub use real::{acquire, forward_scan_request, start_scan_request_listener};
#[cfg(target_os = "macos")]
pub use macos::{acquire, forward_scan_request, start_scan_request_listener};
#[cfg(not(any(windows, target_os = "macos")))]
pub use mock::{acquire, forward_scan_request, start_scan_request_listener};

#[cfg(windows)]
mod real {
    use super::Path;
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
    use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject, INFINITE};

    const MUTEX_NAME: &str = "Global\\CLV3000_SingleInstance_Mutex";
    /// 具名 Event：副实例 `SetEvent` 唤醒主实例的监听线程去读转发文件。不用具名
    /// 管道是因为不需要真正的双向字节流——一次只转发"最新一条待处理路径"，一个
    /// 文件 + 一个信号量级的事件已经足够，比手写 `CreateNamedPipeW`/
    /// `ConnectNamedPipe` 读写循环简单得多。
    const EVENT_NAME: &str = "CLV3000_ScanRequestEvent";

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

    /// 转发文件路径：跟 `app_data_dir()` 而不是系统临时目录同目录，避免多用户
    /// 场景下临时目录权限/清理策略带来的意外。
    fn request_file() -> std::path::PathBuf {
        crate::paths::app_data_dir().join("scan_request.txt")
    }

    fn open_or_create_event() -> Option<HANDLE> {
        let name = HSTRING::from(EVENT_NAME);
        // `bManualReset=false`（自动重置）：监听线程 `WaitForSingleObject` 醒来后
        // 系统自动把事件复位成未触发，不用手动 `ResetEvent`，下一次 `SetEvent`
        // 才会再唤醒一次——语义上刚好是"有新请求就唤醒一次"。
        // SAFETY: 无自定义安全属性，名字/初始状态都是常量，多进程可并发创建/打开
        // 同名 Event 对象（Win32 保证这种情况下返回同一个底层对象）。
        unsafe { CreateEventW(None, false, false, &name) }.ok()
    }

    /// 副实例调用：把扫描路径写进转发文件、`SetEvent` 唤醒主实例的监听线程。
    /// 最佳努力——事件对象打不开（极端情况）就静默失败，调用者（`main.rs`）此时
    /// 应该退回"已经在运行"提示，不会导致用户点了右键菜单毫无反馈。
    pub fn forward_scan_request(path: &Path) -> bool {
        let Ok(_) = std::fs::write(request_file(), path.to_string_lossy().as_bytes()) else {
            return false;
        };
        let Some(event) = open_or_create_event() else {
            return false;
        };
        // SAFETY: `event` 是刚打开的有效句柄。
        unsafe { SetEvent(event) }.is_ok()
    }

    /// 主实例调用：起一个后台线程阻塞等待具名 Event，被唤醒就读转发文件、清空、
    /// 推进 `wakeup` 的全局队列。必须在 `wakeup::init()` 之后、且不依赖 eframe 是否
    /// 已启动——Windows tray-only 下进程可能长期停在 `main::wait_in_tray` 里。
    pub fn start_scan_request_listener() {
        let Some(event) = open_or_create_event() else {
            return;
        };
        // `HANDLE` 内部是 `*mut c_void`，裸指针默认不是 `Send`——但这只是一个不透明的
        // 内核对象句柄（一个整数），不是真的要跨线程共享指向的内存，发送裸整数值
        // 过去、在新线程里用同一个数值重建 `HANDLE` 是安全的。
        let event_value = event.0 as isize;
        std::thread::spawn(move || {
            let event = HANDLE(event_value as *mut _);
            loop {
                // SAFETY: `event` 句柄在这个线程的生命周期内保持有效（永不 close，
                // 随进程退出自动释放，跟 Mutex 句柄的处理方式一致）。
                let _ = unsafe { WaitForSingleObject(event, INFINITE) };
                let path = request_file();
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let trimmed = content.trim();
                    if !trimmed.is_empty() {
                        crate::wakeup::push_scan_request(std::path::PathBuf::from(trimmed));
                    }
                }
                let _ = std::fs::remove_file(&path);
            }
        });
    }
}

/// macOS / Unix：用 Unix 域 socket 充当文件锁 + 转发通道。绑定成功 = 拿到锁；
/// 连接已绑定的 socket 既用来判断"上一个实例是否还活着"，也用来（可选）带一段
/// 扫描路径过去。
#[cfg(target_os = "macos")]
mod macos {
    use super::Path;
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::OnceLock;

    const SOCK_NAME: &str = "clv3000.sock";

    /// 绑定成功后的监听 socket，长期保活（原来是 `mem::forget`，现在存进静态里
    /// 是为了 `start_scan_request_listener` 能拿到它去起 accept 循环）。
    static LISTENER: OnceLock<UnixListener> = OnceLock::new();

    fn sock_path() -> std::path::PathBuf {
        crate::paths::app_data_dir().join(SOCK_NAME)
    }

    /// 如果已经有一个实例在跑，返回 `false`（调用者应该直接退出并提示用户）。
    pub fn acquire() -> bool {
        // 开发调试用：设置 `CLV3000_ALLOW_MULTIPLE_INSTANCES` 可绕过单实例锁，
        // 强制启一个新实例（方便对比/测试新构建，而不必先揪出旧实例）。
        if std::env::var_os("CLV3000_ALLOW_MULTIPLE_INSTANCES").is_some() {
            return true;
        }
        let dir = crate::paths::app_data_dir();
        let _ = std::fs::create_dir_all(&dir);
        let sock = sock_path();

        match UnixListener::bind(&sock) {
            Ok(listener) => {
                let _ = LISTENER.set(listener);
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
                        let _ = LISTENER.set(listener);
                        true
                    }
                    Err(_) => false,
                }
            }
        }
    }

    /// 副实例调用：单独发起一次连接，写入路径字节后关闭（关闭即 EOF，主实例的
    /// accept 循环读到 EOF 就知道这条消息发完了）。跟 `acquire()` 里判断"是否已有
    /// 实例"的那次连接是独立的两次 `connect`，简单，不共享状态。
    pub fn forward_scan_request(path: &Path) -> bool {
        let Ok(mut stream) = UnixStream::connect(sock_path()) else {
            return false;
        };
        stream.write_all(path.to_string_lossy().as_bytes()).is_ok()
    }

    /// 主实例调用：从 `acquire()` 绑定好的监听 socket 起一个 accept 循环线程，把
    /// 每个连接读到的字节当路径转发进 `wakeup` 的全局队列。
    pub fn start_scan_request_listener() {
        let Some(listener) = LISTENER.get() else {
            return;
        };
        let Ok(listener) = listener.try_clone() else {
            return;
        };
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = String::new();
                if stream.read_to_string(&mut buf).is_ok() {
                    let trimmed = buf.trim();
                    if !trimmed.is_empty() {
                        crate::wakeup::push_scan_request(std::path::PathBuf::from(trimmed));
                    }
                }
            }
        });
    }
}

/// 第二个实例启动时调用：弹一个原生提示，告诉用户"已经在运行、请先退出旧实例"，
/// 避免静默退出让人误以为 `cargo run` 没生效、反复点到旧（可能有 bug 的）实例上。
#[cfg(target_os = "macos")]
pub fn notice_already_running() {
    let script = "display alert \"CLV3000 已经在运行\" message \
        \"CLV3000 的一个实例已在运行(通常在菜单栏托盘中)。请先在托盘菜单退出旧实例，\
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
    use super::Path;

    pub fn acquire() -> bool {
        true
    }

    pub fn forward_scan_request(_path: &Path) -> bool {
        false
    }

    pub fn start_scan_request_listener() {}
}
