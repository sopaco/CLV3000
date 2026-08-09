//! 单实例保护：用一个具名 Mutex 防止程序被多次启动。
//! 拿到的 HANDLE 故意不 close——让它随进程退出自动释放即可。
//!
//! 非 Windows（macOS/Linux 开发机预览用）：具名 Mutex 是 Win32 概念，这里直接放行，
//! 不做单实例限制——反正只是拿来看 UI，不是真的部署场景。

#[cfg(windows)]
pub use real::acquire;
#[cfg(not(windows))]
pub use mock::acquire;

#[cfg(windows)]
mod real {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;

    const MUTEX_NAME: &str = "Global\\CLV3000_SingleInstance_Mutex";

    /// 如果已经有一个实例在跑，返回 `false`（调用者应该直接退出）。
    pub fn acquire() -> bool {
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

#[cfg(not(windows))]
mod mock {
    pub fn acquire() -> bool {
        true
    }
}
