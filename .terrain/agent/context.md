---
type: agent_context
project: clv3000
title: Agent Architecture Context
source: .
---

## Project Overview

CLV3000 is a portable, on-demand antivirus scanner for Windows and macOS, written in Rust and powered by ClamAV. It targets emergency and rescue scenarios: run from a USB stick without installation, scan when suspicious activity appears, then quarantine or ignore findings. Consumers are end users, IT technicians, and offline rescue operators invoking `--scan-path` from WinPE or scripts.

Key constraints: **portable-first** path resolution relative to the executable; **pipeline architecture** (enumerate → pre-filter → ClamAV subprocess → UI events); **non-blocking GUI** via background threads and `ScanEvent` channels; **small release binaries** (`opt-level = "s"`, LTO, strip). Not a real-time/on-access AV suite — scanning is user-initiated only.

## Architecture

| Container | Layer | Responsibility |
|-----------|-------|----------------|
| App UI | Presentation + Application | egui pages, `ScanPageState`, settings, virus DB view |
| Scan Engine | Domain | Path enumeration, prescan cache/signatures, ClamAV wrapper |
| Threat Management | Domain | Quarantine, ignore lists, restore |
| Persistence | Infrastructure | TOML config, TSV scan cache, path resolution |
| Platform Layer | Infrastructure | Tray, single-instance IPC, autostart, context menu |
| ClamAV | External subprocess | `clamscan` / `freshclam` bundled beside executable |

**Patterns:** scan pipeline with typed events; `ScanPhase` state machine (Idle → Enumerating → Scanning → Done); `#[cfg(...)]` platform strategy with Linux mocks for UI preview; single-instance lock with scan/show request forwarding.

**Major dependencies:** eframe/egui 0.36 (glow), sysinfo, walkdir, blake3, muda/tray-icon, windows crate (Win32), objc2 (macOS).

```
User/CLI → lifecycle (main.rs) → App UI ←ScanEvent— Scan thread
                                      ↓              ↓
                              config/quarantine   quick_scan/full_scan
                                                       ↓
                                              engine (prescan → clamscan)
```

## Module Map

| Module | Responsibility | Primary paths |
|--------|----------------|---------------|
| app | UI shell, page routing, scan orchestration, settings | `src/app/`, `src/app/app_shell.rs`, `src/app/pages/` |
| app/core | Shared state: scan phase, settings, virus DB status | `src/app/core/` |
| scan | Enumeration, engine pipeline, content cache, Authenticode skip | `src/scan/` |
| quarantine | Move threats to isolated store; restore | `src/quarantine.rs` |
| persistence | User config (TOML) and portable path layout | `src/config.rs`, `src/paths.rs` |
| lifecycle | CLI parsing, startup modes, run/tray transitions | `src/lifecycle.rs`, `src/main.rs` |
| platform | Tray, IPC, autostart, Explorer context menu, wake-up | `src/tray.rs`, `src/single_instance.rs`, `src/autostart.rs`, `src/context_menu.rs`, `src/wakeup.rs` |
| system-integration | ClamAV discovery, signature update (freshclam) | `src/app/freshclam.rs`, `src/clamav_info.rs` |
| ui-infra | Theme, widgets, resource monitor, platform chrome | `src/theme.rs`, `src/widgets.rs`, `src/sysmon.rs`, `src/windows_chrome.rs` |
| build/packaging | macOS bundle script, Windows resources | `build.rs`, `scripts/bundle-macos.sh` |

## Core Flows

### 1. Application Startup

1. Parse CLI (`--tray-only`, `--show`, `--scan-path`, Windows-only `--force-quarantine`).
2. Attempt `single_instance::acquire`; if lock held, forward scan/show request and exit.
3. Initialize wake-up listeners and build system tray.
4. Resolve `InitialMode` — on Windows tray-only, run pre-eframe Win32 message loop (`wait_in_tray`) to avoid flash window.
5. Start `eframe::run_native` with `App`; optional immediate path scan from CLI.

### 2. Quick Scan

1. User triggers Quick Scan; `ScanPageState` spawns background thread with cancel flag.
2. `quick_scan::run` snapshots PIDs, enumerates loaded modules/DLLs, deduplicates paths.
3. Emits `ScanEvent::Enumerating` / `ScanStarted`; passes `PathSource::InMemory` to engine.
4. `engine::prescan` parallel-checks blake3 cache and code signatures; cache hits emit instant `FileScanned`.
5. Remaining paths batched to temp file list; single `clamscan` subprocess spawned; stdout parsed to events.
6. UI applies events via `apply_scan_event`; `ScanPhase` transitions to Done; threats available for quarantine/ignore.

### 3. Explorer / Second-Instance Scan Forwarding

1. Explorer launches `clv3000 --scan-path=<file>` while instance already running.
2. Second process fails `acquire()`, writes path to IPC (`scan_request.txt` + event on Windows; Unix socket on macOS).
3. Running instance's listener wakes UI via `wakeup::request_repaint`.
4. Primary `App` receives path, navigates to scan page, starts path-targeted scan.

### 4. Full Scan

1. User starts Full Scan; optional removable drives per `AppConfig.scan_removable_drives`.
2. `full_scan::run` walks fixed drives, streams executable paths to temp file via `WalkListWriter` (avoids holding 100K+ paths in RAM).
3. Emits `WalkProgress` during enumeration; then same engine prescan → single clamscan batch as quick scan.
4. Results persisted to config (`last_full_scan`) and TSV cache for future rescans.

## Tech Stack

- **Language:** Rust 2024 edition (v0.7.7)
- **GUI:** eframe 0.36 + egui 0.36 (glow backend, default fonts)
- **Scan engine:** ClamAV subprocess (`clamscan`, `freshclam`) — not libclamav FFI
- **Process enumeration:** sysinfo; Windows Toolhelp32 via `windows` crate
- **Signature pre-filter:** WinVerifyTrust (Windows) / codesign (macOS) in `src/scan/authenticode.rs`
- **Content cache:** blake3 hashes, flat TSV files with DB-revision TTL
- **Persistence:** TOML (`serde` + `toml`) for config; TSV for scan cache
- **System integration:** tray-icon, muda menus; Win32 registry; macOS LaunchAgents + objc2
- **Release:** `opt-level = "s"`, LTO, `panic = "abort"`, strip; winresource for Windows metadata
- **Dev infra:** Terrain knowledge in `.terrain/`; CodeGraph index in `.codegraph/`

## System Boundaries

| Boundary | Actor / System | Interface | Trust |
|----------|----------------|-----------|-------|
| ClamAV engine | External subprocess | Spawn `clamscan`/`freshclam`; bundled under `<exe>/clamav/` or macOS `Contents/Resources/clamav/` | Trusted scan authority; missing engine → UI error, no crash |
| Local filesystem | OS | Read scan targets; write quarantine dir, cache TSV, temp file lists | User-owned files; errors surfaced as toasts/`ScanEvent::Error` |
| User config | `%APPDATA%/CLV3000/config.toml` (Win) or `~/Library/Application Support/CLV3000/` (macOS) | TOML: ignored/quarantined records, scan history, settings | App-owned; no encryption |
| CLI | User / Explorer / scripts | `--tray-only`, `--show`, `--scan-path`, `--force-quarantine` (internal Win UAC helper) | `--force-quarantine` not for direct user use |
| Single-instance IPC | Second process | Mutex + event + `scan_request.txt` (Win); `clv3000.sock` (macOS) | Same-user local only |
| OS registry / LaunchAgents | Settings page toggles | Autostart (`--tray-only`); Explorer context menu (Win) | Requires user consent via Settings |
| Environment | Developer only | `CLV3000_ALLOW_MULTIPLE_INSTANCES` bypasses lock | Debug only |

**Out of scope:** network/cloud scanning, always-on real-time protection, Linux production scanning (Linux builds use mocks for UI dev).

## Code Map Index

| Concept | Location | Notes |
|---------|----------|-------|
| Process entry & eframe bootstrap | `src/main.rs` | Tray-only loop, viewport, force-quarantine helper |
| CLI flags & run modes | `src/lifecycle.rs` | `InitialMode`, `RunMode`, `Lifecycle` |
| Portable path resolution | `src/paths.rs` | `exe_dir`, `clamscan_path`, database dir |
| User settings | `src/config.rs` | `AppConfig`, ignored/quarantined arrays |
| Scan event types | `src/scan/mod.rs` | `ScanEvent`, `ScanKind`, `Threat`, `PathSource` |
| ClamAV subprocess wrapper | `src/scan/engine.rs` | prescan, batch clamscan, cancel watchdog |
| Content-hash cache | `src/scan/cache.rs` | `ScanCache`, immutable `CacheSnapshot` for parallel prescan |
| Code signature skip | `src/scan/authenticode.rs` | Platform-specific trusted publisher check |
| Quick scan enumeration | `src/scan/quick_scan.rs` | Running processes + loaded modules |
| Full scan walk | `src/scan/full_scan.rs` | Drive walk, streaming path list |
| Scan UI state machine | `src/app/core/scan_state.rs` | `ScanPhase`, `apply_scan_event`, `threats_modal_open` flag |
| Threat list modal | `src/app/pages/scan.rs` | `threats_modal`, `threats_entry_button`; centered modal replacing inline list |
| App shell & navigation | `src/app/app_shell.rs` | `App`, page routing |
| Scan / dashboard pages | `src/app/pages/` | User-facing scan controls and results |
| Virus DB update UI | `src/app/freshclam.rs`, `src/app/core/virus_db.rs` | freshclam orchestration |
| Quarantine actions | `src/quarantine.rs` | Move, restore, Windows elevation path |
| Single-instance IPC | `src/single_instance.rs` | acquire, forward_scan_request |
| Tray & wake-up | `src/tray.rs`, `src/wakeup.rs` | Event-driven UI repaint from tray/IPC |
| Windows Explorer menu | `src/context_menu.rs` | Registry shell extension |
| Autostart registration | `src/autostart.rs` | Registry (Win) / LaunchAgent (macOS) |