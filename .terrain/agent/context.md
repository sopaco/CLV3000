---
type: agent_context
project: clv3000
title: Agent Architecture Context
source: .
---

## Project Overview

CLV3000 is a **portable on-demand virus-scanning desktop app** for Windows and macOS, written in Rust and backed by ClamAV—a modern homage to KV3000. Positioning: plug-in-a-USB-stick, no-install, low-resource emergency scanner—no real-time protection, scanning only. Three scan entry points (quick/full/context-menu single file) plus full threat handling (ignore/quarantine/restore/delete) and desktop integration (tray, autostart, context menu, single instance). Tray adds **Optimize PC** to launch the CLV3000 Plus companion or open its GitHub Releases page. Key constraints: old-PC friendly (release size opts `opt-level="s"` + lto), GUI event loop sleeps when idle (zero heartbeat), hide-to-tray releases GPU textures and trims working set, scanning via `clamscan` child process not in-process engine. Windows/macOS are real behavior; other targets use a mock engine (UI preview only).

## Architecture

- **UI shell (egui/eframe main thread)**: `App` in `src/app/` holds all UI state, dispatches pages via `Page` enum; split into `app_shell.rs` (`ui()`/`logic()`, resource load/release) and `lifecycle_view.rs` (tray polling, scan/show request forwarding, `hide_to_tray`/window coordination); `logic()` polls tray/show/scan requests, samples sysmon, and uses repaint-on-scan wakeup; macOS scanning holds `ScanActivity` to keep progress animation alive.
- **Scan backend (background threads)**: `src/scan/` uses `std::thread` + `mpsc::Sender<ScanEvent>` for progress/threat events; UI `try_recv` each frame; cancel via atomic `CancelFlag`.
- **Desktop integration layer**: tray, single instance, autostart, context menu, macOS reopen/tray mode—independent modules with per-platform `cfg` implementations.
- **Companion launcher**: `src/clv3000_plus.rs`—tray **Optimize PC** tries local CLV3000 Plus install, else opens Releases in the default browser.
- **State & config**: `AppCore` (page + three page states) and `AppConfig` (TOML: ignore/quarantine/scan history). Page states hold background thread handles and event receivers.
- **Real/mock dual track**: same function signatures; `cfg(windows)` / `cfg(target_os="macos")` for real impl; other targets use synthetic data (~342 processes, ~3000 files, alternating OK/FOUND).

```
┌─────────────────────────── egui/eframe main thread ───────────────────────────┐
│ App (app_shell.rs + lifecycle_view.rs) ─┬─ AppCore ─┬─ ScanPageState          │
│  ui()/logic()/tray/资源/window coord     │          │   (quick/full)          │
│  release_ui_resources()                 │          ├─ VirusDbState           │
│                                         │          └─ SettingsState          │
└──┬──────────┬─────────────┬────────────┘                                      │
   │ wakeup   │ mpsc<ScanEvent> / CancelFlag                                     │
   ▼          ▼             ▼                                                   │
后台线程：scan/quick_scan · scan/full_scan · scan/engine(clamscan child)         │
└───────────────────────────────────────────────────────────────────────────────┘
```

## Module Map

| Module | Responsibility | Primary paths |
|------|------|----------|
| Entry / startup | CLI parsing (`--tray-only`/`--scan-path`/`--show`), viewport build, tray init | `src/main.rs`、`src/lifecycle.rs` |
| UI shell | App assembly, event loop, page dispatch, tray/request polling, resource lifecycle | `src/app/app_shell.rs`、`src/app/lifecycle_view.rs` |
| Pages | Dashboard, quick/full scan, virus DB, settings | `src/app/pages/*`、`src/app/settings.rs` |
| Window chrome | Sidebar, resource bar, title bar (non-Windows), Toast | `src/app/chrome.rs`、`src/widgets.rs`、`src/theme.rs` |
| Core state | `AppCore`, scan state machine (Idle/Enumerating/Scanning/Done), settings, virus DB state | `src/app/core/*` |
| Scan orchestration | Prescan (blake3 hash + cache lookup), clamscan batching, result parse, cache write | `src/scan/engine.rs`、`src/scan/cache.rs` |
| Quick scan | Process/module enumeration (Win Toolhelp32 / macOS sysinfo) | `src/scan/quick_scan.rs` |
| Full scan | Drive walk, executable/Mach-O filtering, streaming temp walklist | `src/scan/full_scan.rs`、`src/scan/mod.rs` |
| Signature prefilter | WinVerifyTrust (PE/catalog), macOS codesign—skip trusted signatures | `src/scan/authenticode.rs` |
| Virus DB management | `freshclam` child update, engine/DB version probe | `src/app/freshclam.rs`、`src/clamav_info.rs` |
| Threat handling | Quarantine/restore/delete, forced quarantine (kill holders + elevated move) | `src/quarantine.rs` |
| CLV3000 Plus launcher | Tray **Optimize PC**: launch companion app or open Releases page | `src/clv3000_plus.rs` |
| Desktop integration | Tray, single instance, autostart, context menu, macOS reopen/tray mode | `src/tray.rs`、`src/single_instance.rs`、`src/autostart.rs`、`src/context_menu.rs`、`src/macos_reopen.rs` |
| Infrastructure | Path resolution, config persistence, sysmon, icon assets | `src/paths.rs`、`src/config.rs`、`src/sysmon.rs`、`src/icon_data.rs`、`src/icons.rs` |

## Core Flows

**Quick scan / full scan (background task)**
1. Page start → create `CancelFlag` + `mpsc` channel, `spawn` background thread (quick→`quick_scan::run`, full→`full_scan::run`).
2. Enumeration (quick: enumerate processes+modules deduped; full: walk fixed drives, stream executables into temp walklist); UI shows "N files found". macOS full scan: positive extension whitelist + `is_collectable_macho` (MH_EXECUTE/DYLIB/BUNDLE only), prune `DerivedData`/`/.git`, skip cache/log trees and duplicate `/Volumes` symlinks.
3. Engine phase: multithreaded prescan—blake3 content hash + `ScanCache` lookup (invalidated on DB version change); cache miss → assemble `clamscan` child batches, parse OK/FOUND, write cache.
4. Progress/current path/threats via `ScanEvent`; UI `try_recv` each frame, `apply_scan_event` advances state machine.
5. Cancel: set `CancelFlag`, backend kills child process and exits; on finish return scan count/elapsed summary.

**Context-menu "Scan with CLV3000" (cold start / forward)**
1. Shell starts second instance → `single_instance::acquire` probes running instance (Win named Mutex / Unix socket).
2. Instance exists → forward via `wakeup::push_scan_request` (path) or `forward_show_request` (`--show`); this instance exits.
3. No instance → parse `--scan-path`, enter FullScan page and `start_path` scan.

**`--show` / surface main window**
1. CLI `--show` or Dock click while hidden in tray → `single_instance::forward_show_request` (Win named Event / macOS socket `__CLV3000_SHOW__` marker).
2. Primary instance listener (`start_request_listeners`) pushes `wakeup::push_show_request` and `ping()`.
3. `wait_in_tray` or `App::logic` drains `show_requests` → `request_show_window` (`RunMode::ShowWindow`, macOS `leave_tray_mode`).

**Lifecycle & tray**
1. Startup dispatches `InitialMode` (ShowWindow / TrayOnly / QuickScan / About / ScanPath); `--tray-only` hides window; `--show` overrides tray-only on cold start.
2. Close button / non

-`RunMode::Quit` → cancel close, `hide_to_tray` (release sysmon, clear egui caches/textures, `trim_working_set`, macOS `enter_tray_mode`).
3. Tray menu: Show / Quick Scan / **Optimize PC** (`clv3000_plus::launch_or_open_releases`) / About / Quit.
4. While scanning, `request_repaint_after(250–500ms)` for progress; idle = zero repaint, event loop sleeps.

**Threat handling loop**
1. Threat detected → list on page; user may ignore (write `AppConfig.ignored`, skip on rescan) or quarantine (move to quarantine dir).
2. File locked / permission denied → Windows forced quarantine: enumerate holders, kill, elevated move if needed.
3. Settings quarantine list: restore (move back) or permanent delete; all persisted via `AppConfig` (TOML).

## Tech Stack

- **Language / version**: Rust, edition 2024; release size-optimized (`opt-level="s"`, `lto`, `panic="abort"`, `strip`).
- **GUI**: egui / eframe 0.36 (glow renderer, default fonts, non-default features); no heavy custom UI deps.
- **Windows native**: `windows` crate 0.62 (Toolhelp, WinTrust, Shell, registry, process mgmt); `winresource` in build.rs embeds three ico resources: main `icon_app`=ID 1, tray `icon_tray`=ID 2, extension pack `icon_expack_1`=ID 3.
- **macOS native**: `objc2` family (AppKit/Foundation); `block2` for notification blocks; `macos_reopen.rs` handles tray mode, Dock reopen, `ScanActivity` during scans.
- **Tray / menus**: `tray-icon` + `muda`.
- **Scan external deps**: ClamAV portable dir (`clamscan`/`freshclam` + `database/*.cvd`), child-process only—**not** in-process engine loading.
- **Helpers**: `sysinfo` (resource monitor), `blake3` (file gene hash), `walkdir` (disk walk), `serde`+`toml` (config), `image` (icon decode), `directories` (dir resolution).

## System Boundaries

- **External process contracts**: `clamscan` (scan/`--version`), `freshclam` (update/`--datadir`) beside exe `clamav/` or PATH; missing engine → UI "engine not found" without crash. CLV3000 Plus: Win `clv3000-plus.exe` beside exe, macOS `/Applications/CLV3000 Plus.app`; launched via tray **Optimize PC** or browser opens `https://github.com/sopaco/CLV3000-Plus/releases` if absent.
- **Filesystem**: exe-adjacent `clamav/database/` (signatures); AppData `config.toml`, scan cache, quarantine dir; full-scan temp walklist in system temp.
- **Registry**: autostart Run key, context-menu `ShellEx`/verb keys (menu `Icon` `"<exe>,-2"` references embedded `icon_tray` ID 2); single-instance named Mutex; scan/show named Events (`CLV3000_ScanRequestEvent`, `CLV3000_ShowRequestEvent`).
- **macOS autostart**: LaunchAgent plist (`RunAtLoad` + `Interactive`), `/usr/bin/open -a <bundle> --args --tray-only` via Launch Services for proper GUI/tray; single-instance Unix socket with `__CLV3000_SHOW__` marker for show-window IPC.
- **Network**: `freshclam` signature updates (ClamAV servers, no app HTTP stack); browser open for CLV3000 Plus Releases when companion not installed.
- **Trust boundary**: quarantine/forced quarantine may kill holders and trigger UAC elevation—high risk; signature prefilter trusts system cert chains; mock mode (non Win/mac) is synthetic UI preview only.

## Code Map Index

| Concept | Location | Notes |
|------|------|------|
| Entry, CLI modes | `src/main.rs`、`src/lifecycle.rs` | `InitialMode`/`RunMode`; `parse_show` for `--show` |
| App shell, event loop | `src/app/app_shell.rs`、`src/app/lifecycle_view.rs` | `ui()`/`logic()`; `hide_to_tray`/`reconcile_lifecycle`/tray + scan/show polling |
| Pages & chrome | `src/app/pages/`、`src/app/chrome.rs`、`src/app/settings.rs` | `Page` dispatch; `settings_page` in settings.rs |
| Core state | `src/app/core/mod.rs`、`scan_state.rs`、`settings_state.rs`、`virus_db.rs` | `AppCore`、`ScanPhase`、`apply_scan_event` |
| Scan orchestration | `src/scan/engine.rs`、`src/scan/cache.rs` | Prescan, clamscan batching, cache |
| Quick / full scan | `src/scan/quick_scan.rs`、`src/scan/full_scan.rs` | Process enum / drive walk; macOS `is_collectable_macho` in `scan/mod.rs` |
| Signature prefilter | `src/scan/authenticode.rs` | WinVerifyTrust / codesign |
| Virus DB update | `src/app/freshclam.rs`、`src/clamav_info.rs` | `freshclam` child, version parse |
| Threat handling | `src/quarantine.rs`、`src/config.rs` | Quarantine/restore/forced quarantine |
| CLV3000 Plus launcher | `src/clv3000_plus.rs` | Tray **Optimize PC**; `launch_or_open_releases` |
| Tray / single instance / autostart / context menu | `src/tray.rs`、`src/single_instance.rs`、`src/autostart.rs`、`src/context_menu.rs` | Tray menu includes `optimize_pc`; `forward_show_request`; `start_request_listeners` |
| Wakeup & macOS reopen | `src/wakeup.rs`、`src/macos_reopen.rs` | `push_show_request`/`show_requests`; `enter_tray_mode`/`leave_tray_mode`/`install_reopen_handler` |
| Paths / config / monitor | `src/paths.rs`、`src/config.rs`、`src/sysmon.rs` | ClamAV dir resolve, TOML persistence, resource sampling |
| Theme / icons / widgets | `src/theme.rs`、`src/icons.rs`、`src/icon_data.rs`、`src/widgets.rs`、`src/about_dialog.rs` | Design tokens, icons, shared controls |