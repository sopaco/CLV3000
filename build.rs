//! 给 Windows 版 exe 嵌入图标资源（RT_ICON），也就是双击运行前在资源管理器/
//! 桌面上看到的那个文件图标——这个跟 `App::new` 里用 `icon_data::load_app_icon`
//! 设的"窗口图标"是两件不同的事：窗口图标是程序跑起来之后 eframe/winit 通过
//! Win32 API 动态设置的，只在程序运行时的标题栏/任务栏生效；exe 文件本身的图标
//! 是链接期写进 PE 资源段的静态数据，程序不运行的时候（资源管理器里看文件、
//! 快捷方式）也看得到，必须在编译时用 `winresource` 这类工具把 `.ico` 嵌进去，
//! 运行期的任何代码都做不到这件事。
//!
//! Windows 的 exe 图标资源只认 `.ico`（多分辨率打包格式），不能直接拿 `.png`
//! 用——`assets/icons/icon_app.ico` 是从 `icon_app.png` 转出来的多尺寸 ICO
//! （16/24/32/48/64/128/256），转换后的产物直接提交进仓库，不在构建时现转，
//! 省得给构建加一个 image-to-ico 的依赖。
//!
//! **两个容易踩的交叉编译坑，都在这个文件里躲过了：**
//!
//! 1. `Cargo.toml` 里 `winresource` 故意没写成
//!    `[target.'cfg(windows)'.build-dependencies]`——build-dependencies 是在
//!    构建这台机器（host）上编译运行的，Cargo 对 `[target.cfg(...)]` 里的
//!    build-dependencies 是按 **host** 平台匹配 cfg 谓词，不是按 `--target`
//!    指定的最终目标。在非 Windows host 上交叉编译 Windows 目标时，那样写会
//!    导致 `winresource` 被直接从构建图里剔除，编译期没有任何报错，图标就是
//!    嵌不上——所以 `Cargo.toml` 里它是无条件的 `[build-dependencies]`。
//!
//! 2. 这个文件里判断"当前是不是在给 Windows 编译"，用的是
//!    `CARGO_CFG_TARGET_OS` 环境变量，**不是** `#[cfg(windows)]`——`#[cfg(...)]`
//!    写在 `build.rs` 源码里，编译的是这个构建脚本本身，构建脚本永远在 host 上
//!    编译并运行，所以 `#[cfg(windows)]` 在这里问的是"host 是不是 Windows"，
//!    根子上就问错了问题。`CARGO_CFG_TARGET_OS` 才是 Cargo 传给构建脚本、
//!    反映"这次真正要编译给哪个目标平台"的信息，这是 Cargo 官方文档里指定的
//!    标准做法。

const ICON_REL: &str = "assets/icons/icon_app.ico";

fn main() {
    println!("cargo:rerun-if-changed={ICON_REL}");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        // 非 Windows 目标不嵌图标（PE 图标资源只在 Windows 有意义），直接跳过。
        // 交叉编译带图标的 exe 的方法见本文件顶部文档注释，这里不再每次 `cargo check`
        // 都刷一条 warning 干扰输出。
        return;
    }

    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set by cargo");
    let icon_path = format!("{manifest_dir}/{ICON_REL}");
    if !std::path::Path::new(&icon_path).exists() {
        panic!("Windows 图标文件不存在：{icon_path}");
    }

    let mut res = winresource::WindowsResource::new();
    configure_cross_toolchain(&mut res);
    // 绝对路径：避免 windres/rc.exe 因工作目录不同而找不到相对路径里的 .ico。
    res.set_icon(&icon_path);

    if let Err(e) = res.compile() {
        let target = std::env::var("TARGET").unwrap_or_default();
        let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
        panic!(
            "嵌入 Windows exe 图标失败：{e}\n\
             目标：{target} ({target_env})\n\
             图标：{icon_path}\n\
             常见原因：\n\
             - GNU 工具链：未安装 mingw-w64，或 windres/ar 不在 PATH（macOS: brew install mingw-w64）\n\
             - MSVC 工具链：未安装 Windows SDK（缺少 rc.exe），或交叉编译时未安装 llvm-rc\n\
             - 可设置环境变量 WINDRES / AR / RC_PATH 指向正确的资源编译器"
        );
    }
}

/// 交叉编译到 Windows GNU 时，把 windres/ar 指到带 target 前缀的工具，并优先用
/// gcc-ar（与 `.cargo/config.toml` 里的 linker/ar 配置一致）。
fn configure_cross_toolchain(res: &mut winresource::WindowsResource) {
    let host = std::env::var("HOST").unwrap_or_default();
    let target = std::env::var("TARGET").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    if host == target || target_env != "gnu" {
        return;
    }

    let prefix = match target.as_str() {
        "x86_64-pc-windows-gnu" | "x86_64-pc-windows-msvc" => "x86_64-w64-mingw32-",
        "i686-pc-windows-gnu" | "i686-pc-windows-msvc" => "i686-w64-mingw32-",
        "aarch64-pc-windows-gnu" | "aarch64-pc-windows-gnullvm" => "aarch64-w64-mingw32-",
        _ => return,
    };

    if std::env::var("WINDRES").is_err() {
        let windres = format!("{prefix}windres");
        if command_exists(&windres) {
            res.set_windres_path(&windres);
        }
    }

    if std::env::var("AR").is_err() {
        for ar in [format!("{prefix}gcc-ar"), format!("{prefix}ar")] {
            if command_exists(&ar) {
                res.set_ar_path(&ar);
                break;
            }
        }
    }
}

fn command_exists(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
