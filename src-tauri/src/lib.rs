use std::{
    env,
    ffi::{c_void, OsStr},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::TcpStream,
    path::{Component, Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[cfg(windows)]
use std::os::windows::{ffi::OsStrExt, process::CommandExt};

use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

mod capture_window;
mod codedrobe_diagnostic;
#[cfg(debug_assertions)]
mod integration_qa;
mod live_preview;
mod region_theme;
mod theme_compiler;
mod theme_engine;
mod theme_fusion;
mod theme_library;
mod theme_model;
mod workbuddy_session;
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tungstenite::{client, stream::MaybeTlsStream, Message, WebSocket};

// WorkBuddy 5.2.6 can leave the historical port 9336 in a non-responsive state on Windows.
// Keep CDP loopback-only, but use an alternate default that is verified on the current runtime.
const DEFAULT_CDP_PORT: u16 = 19336;
const LEGACY_CDP_PORT: u16 = 9336;
const MIN_SUPPORTED_WORKBUDDY_VERSION: (u16, u16, u16) = (5, 2, 6);
const CODEDROBE_STYLE_ID: &str = "codedrobe-theme-style-workbuddy";
const RENDERER_PATH: &str = "resources/app.asar/renderer/index.html";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MONITOR_INTERVAL: Duration = Duration::from_millis(2_500);
// Status polling runs both from the UI and the background monitor. A regular WorkBuddy launch
// does not expose CDP, so this must fail quickly instead of tying up most of every refresh tick.
const CDP_STATUS_PROBE_TIMEOUT: Duration = Duration::from_millis(300);
const RENDERER_SETTLE_DELAY: Duration = Duration::from_secs(4);
const RETRY_DELAYS_SECS: [u64; 3] = [1, 2, 5];

type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppError {
    code: String,
    message: String,
}

impl AppError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone)]
struct AppState {
    operation_lock: Arc<Mutex<()>>,
    manual_operation_pending: Arc<AtomicBool>,
    monitor: Arc<MonitorRuntime>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            operation_lock: Arc::new(Mutex::new(())),
            manual_operation_pending: Arc::new(AtomicBool::new(false)),
            monitor: Arc::new(MonitorRuntime::default()),
        }
    }
}

#[derive(Default)]
struct MonitorRuntime {
    started: AtomicBool,
    stop_requested: AtomicBool,
    retry_reset_requested: AtomicBool,
    snapshot: Mutex<MonitorSnapshot>,
}

#[derive(Debug, Clone)]
struct MonitorSnapshot {
    status: String,
    retry_count: u8,
    last_auto_recovery_at: Option<String>,
    last_error_code: Option<String>,
}

impl Default for MonitorSnapshot {
    fn default() -> Self {
        Self {
            status: "未监控".to_string(),
            retry_count: 0,
            last_auto_recovery_at: None,
            last_error_code: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ThemeRecord {
    id: String,
    name: String,
    description: String,
    package_path: String,
    preview_path: String,
    verified_work_buddy_version: String,
    theme_version: String,
    runtime_theme_id: String,
    #[serde(default)]
    is_custom: bool,
}

#[derive(Debug, Clone)]
struct ResolvedTheme {
    record: ThemeRecord,
    package_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ThemeControls {
    brightness: i16,
    blur: u8,
    panel_opacity: u8,
    accent: Option<String>,
}

impl Default for ThemeControls {
    fn default() -> Self {
        Self {
            brightness: 0,
            blur: 8,
            panel_opacity: 82,
            accent: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RgbColor {
    red: u8,
    green: u8,
    blue: u8,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum DesiredState {
    Theme,
    Original,
}

impl Default for DesiredState {
    fn default() -> Self {
        Self::Original
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
struct PersistentState {
    schema_version: u32,
    desired_state: DesiredState,
    selected_theme_id: Option<String>,
    selected_theme_version: Option<String>,
    auto_keep_theme: bool,
    workbuddy_path: Option<PathBuf>,
    preferred_port: u16,
    last_successful_apply_at: Option<String>,
    last_successful_verify_theme: Option<String>,
    last_auto_recovery_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ai_settings: Option<Value>,
}

impl Default for PersistentState {
    fn default() -> Self {
        Self {
            schema_version: 2,
            desired_state: DesiredState::Original,
            selected_theme_id: None,
            selected_theme_version: None,
            auto_keep_theme: false,
            workbuddy_path: None,
            preferred_port: DEFAULT_CDP_PORT,
            last_successful_apply_at: None,
            last_successful_verify_theme: None,
            last_auto_recovery_at: None,
            ai_settings: None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacySettings {
    workbuddy_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkBuddyStatus {
    app_found: bool,
    path: Option<String>,
    version: Option<String>,
    running: bool,
    cdp_available: bool,
    renderer_available: bool,
    current_theme_id: Option<String>,
    style_node_count: Option<u32>,
    logs_directory: String,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeStatus {
    desired_state: String,
    selected_theme_id: Option<String>,
    auto_keep_theme: bool,
    monitor_status: String,
    retry_count: u8,
    last_auto_recovery_at: Option<String>,
    last_error_code: Option<String>,
    state_path: String,
    login_autostart_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationResult {
    success: bool,
    action: String,
    theme_id: Option<String>,
    message: String,
    rolled_back: bool,
    style_node_count: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CdpTarget {
    id: String,
    #[serde(rename = "type")]
    target_type: String,
    title: String,
    url: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    websocket_debugger_url: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Clone)]
struct RendererTarget {
    target_id: String,
    websocket_debugger_url: String,
}

#[derive(Debug, Clone)]
struct ThemeNodeMetadata {
    style_node_count: u32,
    runtime_theme_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyMode {
    Launch,
    Attach,
    Restart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyOrigin {
    Manual,
    Automatic,
}

#[derive(Default)]
struct RetryControl {
    key: Option<String>,
    attempts: u8,
    next_attempt: Option<Instant>,
    last_error: Option<String>,
    blocked: bool,
}

impl RetryControl {
    fn reset_if_key_changed(&mut self, key: String) -> bool {
        if self.key.as_deref() != Some(&key) {
            self.reset();
            self.key = Some(key);
            true
        } else {
            false
        }
    }

    fn defer_initial_attempt(&mut self, delay: Duration) {
        self.next_attempt = Some(Instant::now() + delay);
    }

    fn can_attempt(&self) -> bool {
        !self.blocked
            && self.attempts < RETRY_DELAYS_SECS.len() as u8
            && self.next_attempt.is_none_or(|time| Instant::now() >= time)
    }

    fn record_failure(&mut self) {
        self.attempts = self.attempts.saturating_add(1);
        self.next_attempt = RETRY_DELAYS_SECS
            .get(self.attempts.saturating_sub(1) as usize)
            .map(|seconds| Instant::now() + Duration::from_secs(*seconds));
    }

    fn reset(&mut self) {
        self.key = None;
        self.attempts = 0;
        self.next_attempt = None;
        self.last_error = None;
        self.blocked = false;
    }

    fn record_error(&mut self, code: &str) {
        self.record_failure();
        self.last_error = Some(code.to_string());
        self.blocked = codedrobe_diagnostic::deterministic(code);
    }

    fn wait_status(&self) -> (&'static str, Option<&str>) {
        let status = if self.blocked {
            "自动恢复已暂停；请手动应用或更新主题切换器"
        } else if self.attempts >= RETRY_DELAYS_SECS.len() as u8 {
            "已停止自动重试；请检查连接后手动应用"
        } else {
            "等待页面稳定后重试"
        };
        (status, self.last_error.as_deref())
    }
}

pub fn run() {
    let background_mode = launched_in_background_mode();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, arguments, _| {
            if !arguments.iter().any(|arg| arg == "--background") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .manage(AppState::default())
        .manage(workbuddy_session::StudioState::default())
        .setup(move |app| {
            workbuddy_session::initialize_or_block(app.handle());
            #[cfg(debug_assertions)]
            if let Some(root) = env::var_os("STUDIO_QA_ROOT") {
                app.asset_protocol_scope()
                    .allow_directory(PathBuf::from(root), true)?;
            }
            if let Some(window) = app.get_webview_window("main") {
                if background_mode {
                    window.hide()?;
                } else {
                    window.show()?;
                    window.set_focus()?;
                }
            }
            let managed = app.state::<AppState>();
            start_monitor(
                app.handle().clone(),
                managed.operation_lock.clone(),
                managed.manual_operation_pending.clone(),
                managed.monitor.clone(),
                background_mode,
            );
            #[cfg(debug_assertions)]
            integration_qa::start_if_requested(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                workbuddy_session::close_window(window.app_handle());
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            workbuddy_session::studio_library,
            workbuddy_session::studio_import,
            workbuddy_session::studio_latest_draft,
            workbuddy_session::studio_update_draft,
            workbuddy_session::studio_update_design,
            workbuddy_session::studio_save_draft,
            workbuddy_session::studio_open_theme,
            workbuddy_session::studio_copy_legacy,
            workbuddy_session::studio_copy_fusion,
            workbuddy_session::studio_rename_theme,
            workbuddy_session::studio_delete_theme,
            workbuddy_session::studio_cancel_job,
            workbuddy_session::studio_apply,
            workbuddy_session::studio_restore,
            workbuddy_session::studio_set_path,
            workbuddy_session::studio_auto_keep,
            workbuddy_session::studio_preview_theme,
            workbuddy_session::studio_start_trial,
            workbuddy_session::studio_sync_trial,
            workbuddy_session::studio_renew_trial,
            workbuddy_session::studio_capture_trial,
            workbuddy_session::studio_export_capture,
            workbuddy_session::studio_cancel_trial,
            workbuddy_session::studio_resolve_recovery,
            workbuddy_session::studio_confirm_trial,
            workbuddy_session::studio_runtime,
            clear_legacy_ai_credentials,
            open_logs_directory,
        ])
        .run(tauri::generate_context!())
        .expect("error while running WorkBuddy Theme Switcher");
}

fn launched_in_background_mode() -> bool {
    env::args().any(|argument| argument.eq_ignore_ascii_case("--background"))
}

fn runtime_status(app: &AppHandle, state: &AppState) -> AppResult<RuntimeStatus> {
    let saved = load_persistent_state(&app)?;
    let snapshot = state
        .monitor
        .snapshot
        .lock()
        .map_err(|_| AppError::new("MONITOR_STATE_FAILED", "无法读取后台监控状态。"))?
        .clone();
    Ok(RuntimeStatus {
        desired_state: match saved.desired_state {
            DesiredState::Theme => "theme".to_string(),
            DesiredState::Original => "original".to_string(),
        },
        selected_theme_id: saved.selected_theme_id,
        auto_keep_theme: saved.auto_keep_theme,
        monitor_status: snapshot.status,
        retry_count: snapshot.retry_count,
        last_auto_recovery_at: snapshot
            .last_auto_recovery_at
            .or(saved.last_auto_recovery_at),
        last_error_code: snapshot.last_error_code,
        state_path: state_path()?.to_string_lossy().to_string(),
        login_autostart_enabled: login_autostart_enabled(&app),
    })
}

#[tauri::command]
fn set_workbuddy_path(app: AppHandle, path: String) -> AppResult<WorkBuddyStatus> {
    let candidate = PathBuf::from(path);
    validate_workbuddy_path(&candidate)?;
    let mut saved = load_persistent_state(&app)?;
    saved.workbuddy_path = Some(candidate);
    save_persistent_state(&app, &saved)?;
    detect_workbuddy_inner(&app)
}

#[tauri::command]
fn set_auto_keep_theme(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> AppResult<RuntimeStatus> {
    with_manual_operation(&state, || {
        if workbuddy_session::has_trial(&app) {
            return Err(AppError::new(
                "TRIAL_ACTIVE",
                "实机试用或待恢复期间不能更改自动保持。",
            ));
        }
        let mut saved = load_persistent_state(&app)?;
        if enabled
            && (saved.desired_state != DesiredState::Theme || saved.selected_theme_id.is_none())
        {
            return Err(AppError::new(
                "THEME_NOT_SELECTED",
                "请先成功应用一个主题，再开启自动保持。",
            ));
        }
        saved.auto_keep_theme = enabled;
        save_persistent_state(&app, &saved)?;
        set_login_autostart(&app, enabled)?;
        state
            .monitor
            .retry_reset_requested
            .store(true, Ordering::SeqCst);
        update_monitor(
            &state.monitor,
            if enabled {
                "正在等待 WorkBuddy"
            } else {
                "自动保持已暂停"
            },
            0,
            None,
        );
        runtime_status(&app, &state)
    })
}

#[tauri::command]
fn open_logs_directory(app: AppHandle) -> AppResult<()> {
    let directory = log_directory()?;
    fs::create_dir_all(&directory)
        .map_err(|_| AppError::new("LOG_DIRECTORY_FAILED", "无法创建日志目录。"))?;
    let mut command = Command::new("explorer.exe");
    configure_hidden_command(&mut command);
    command
        .arg(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| AppError::new("LOG_DIRECTORY_FAILED", "无法打开日志目录。"))?;
    let _ = app;
    Ok(())
}

fn with_manual_operation<T>(
    state: &AppState,
    operation: impl FnOnce() -> AppResult<T>,
) -> AppResult<T> {
    state.manual_operation_pending.store(true, Ordering::SeqCst);
    let lock = match state.operation_lock.lock() {
        Ok(lock) => lock,
        Err(_) => {
            state
                .manual_operation_pending
                .store(false, Ordering::SeqCst);
            return Err(AppError::new("OPERATION_LOCK_FAILED", "主题操作锁不可用。"));
        }
    };
    let result = operation();
    drop(lock);
    state
        .manual_operation_pending
        .store(false, Ordering::SeqCst);
    result
}

fn detect_workbuddy_inner(app: &AppHandle) -> AppResult<WorkBuddyStatus> {
    let logs_directory = log_directory()?.to_string_lossy().to_string();
    let Some(path) = resolve_workbuddy_path(app)? else {
        return Ok(WorkBuddyStatus {
            app_found: false,
            path: None,
            version: None,
            running: false,
            cdp_available: false,
            renderer_available: false,
            current_theme_id: None,
            style_node_count: None,
            logs_directory,
            message: "未找到 WorkBuddy.exe，可手动选择。".to_string(),
        });
    };
    let version = read_workbuddy_version(&path);
    let running = is_workbuddy_running();
    let port = load_persistent_state(app)?.preferred_port;
    // A missing theme node is normal after a renderer refresh. CDP availability must be based on
    // renderer discovery rather than on the optional metadata read, otherwise automatic recovery
    // incorrectly treats a healthy CDP endpoint as requiring a WorkBuddy restart.
    let renderer_available = running && discover_renderer(port).is_ok();
    let metadata = if renderer_available {
        read_theme_node_metadata(port).ok()
    } else {
        None
    };
    let cdp_available = renderer_available;
    let catalog = theme_catalog(app).unwrap_or_default();
    let current_theme_id = metadata.as_ref().and_then(|item| {
        item.runtime_theme_id.as_ref().map(|runtime_id| {
            catalog
                .iter()
                .find(|theme| theme.runtime_theme_id == *runtime_id)
                .map(|theme| theme.id.clone())
                .unwrap_or_else(|| {
                    theme_library::reference_from_runtime(runtime_id)
                        .map(|r| r.key())
                        .unwrap_or_else(|| runtime_id.clone())
                })
        })
    });
    let message = match (running, cdp_available, current_theme_id.as_deref()) {
        (false, _, _) => "WorkBuddy 未运行；应用或试用前请确认启动。".to_string(),
        (true, false, _) => "WorkBuddy 正在运行，但未开放 CDP；请在切换器中确认重启。".to_string(),
        (true, true, Some(_)) => "已连接 WorkBuddy，检测到 CodeDrobe 主题。".to_string(),
        (true, true, None) => "已连接 WorkBuddy，当前为原版状态。".to_string(),
    };
    Ok(WorkBuddyStatus {
        app_found: true,
        path: Some(path.to_string_lossy().to_string()),
        version,
        running,
        cdp_available,
        renderer_available,
        current_theme_id,
        style_node_count: metadata.map(|item| item.style_node_count),
        logs_directory,
        message,
    })
}

fn apply_theme_locked(
    app: &AppHandle,
    theme_id: &str,
    allow_restart: bool,
    origin: ApplyOrigin,
) -> AppResult<OperationResult> {
    let theme = resolve_theme(app, theme_id)?;
    let status = detect_workbuddy_inner(app)?;
    let workbuddy_path = status
        .path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| AppError::new("WORKBUDDY_NOT_FOUND", "未找到有效的 WorkBuddy.exe。"))?;
    ensure_supported_workbuddy_version(status.version.as_deref())?;
    let port = load_persistent_state(app)?.preferred_port;
    match decide_apply_mode(status.running, status.cdp_available, allow_restart)? {
        ApplyMode::Attach => {}
        ApplyMode::Launch => {
            launch_workbuddy(&workbuddy_path, port)?;
            wait_for_renderer(port)?;
        }
        ApplyMode::Restart => {
            request_graceful_workbuddy_close()?;
            launch_workbuddy(&workbuddy_path, port)?;
            wait_for_renderer(port)?;
        }
    }
    // The monitor has already discovered a live renderer before it reaches this path. Calling a
    // second short-lived probe immediately after its raw metadata check can race Electron's CDP
    // socket during a refresh. CodeDrobe's apply command performs the same compatibility check;
    // retries remain bounded by the monitor. Manual application keeps the explicit readiness probe
    // so the UI can report a loading page before changing its style node.
    if origin == ApplyOrigin::Manual {
        wait_for_theme_probe(app, &theme, port)?;
    }

    // WorkBuddy is launched by this process with CREATE_NO_WINDOW. CodeDrobe only attaches to the
    // renderer afterwards, so neither Node nor CodeDrobe has to create a child WorkBuddy process.
    let args = vec![
        "apply".to_string(),
        "--app".to_string(),
        "workbuddy".to_string(),
        "--theme".to_string(),
        child_process_path_string(&theme.package_path),
        "--app-path".to_string(),
        child_process_path_string(&workbuddy_path),
        "--port".to_string(),
        port.to_string(),
        "--no-launch".to_string(),
        "--json".to_string(),
    ];
    if let Err(error) = run_codedrobe(app, &args) {
        write_log(
            app,
            action_for(origin),
            "failed",
            Some(&theme.record.id),
            status.path.as_deref(),
            status.version.as_deref(),
            &error.code,
        )?;
        return Err(error);
    }

    let verify_args = vec![
        "verify".to_string(),
        "--app".to_string(),
        "workbuddy".to_string(),
        "--theme".to_string(),
        child_process_path_string(&theme.package_path),
        "--port".to_string(),
        port.to_string(),
        "--json".to_string(),
    ];
    let verified = run_codedrobe(app, &verify_args)
        .as_ref()
        .is_ok_and(codedrobe_verify_passes);
    let metadata = read_theme_node_metadata(port);
    let node_ok = metadata.as_ref().is_ok_and(|item| {
        item.style_node_count == 1
            && item.runtime_theme_id.as_deref() == Some(&theme.record.runtime_theme_id)
    });
    if !verified || !node_ok {
        let rollback_succeeded = restore_with_codedrobe(app, port).is_ok();
        let code = if !verified {
            "VERIFY_FAILED"
        } else {
            "STYLE_NODE_COUNT_INVALID"
        };
        write_log(
            app,
            action_for(origin),
            "rolled-back",
            Some(&theme.record.id),
            status.path.as_deref(),
            status.version.as_deref(),
            code,
        )?;
        return Err(AppError::new(
            code,
            if rollback_succeeded {
                "主题验证未通过，已自动恢复 WorkBuddy 原版。"
            } else {
                "主题验证未通过，自动恢复原版也未完成。"
            },
        ));
    }
    write_log(
        app,
        action_for(origin),
        "success",
        Some(&theme.record.id),
        status.path.as_deref(),
        status.version.as_deref(),
        "OK",
    )?;
    Ok(OperationResult {
        success: true,
        action: "apply".to_string(),
        theme_id: Some(theme.record.id),
        message: if origin == ApplyOrigin::Automatic {
            "主题已自动恢复并验证，当前仅存在一个 CodeDrobe 主题节点。".to_string()
        } else {
            "主题已应用并验证，当前仅存在一个 CodeDrobe 主题节点。".to_string()
        },
        rolled_back: false,
        style_node_count: Some(1),
    })
}

fn restore_theme_locked(app: &AppHandle) -> AppResult<OperationResult> {
    let status = detect_workbuddy_inner(app)?;
    if !status.running {
        write_log(
            app,
            "restore",
            "already-native",
            None,
            status.path.as_deref(),
            status.version.as_deref(),
            "NOT_RUNNING",
        )?;
        return Ok(OperationResult {
            success: true,
            action: "restore".to_string(),
            theme_id: None,
            message: "已记录原版状态；WorkBuddy 下次启动时不会自动注入主题。".to_string(),
            rolled_back: false,
            style_node_count: Some(0),
        });
    }
    if !status.cdp_available {
        return Err(AppError::new(
            "RESTART_REQUIRED",
            "WorkBuddy 正在运行但未开放 CDP；请确认重启后再恢复。",
        ));
    }
    let port = load_persistent_state(app)?.preferred_port;
    restore_with_codedrobe(app, port)?;
    let metadata = read_theme_node_metadata(port)?;
    if metadata.style_node_count != 0 {
        write_log(
            app,
            "restore",
            "failed",
            None,
            status.path.as_deref(),
            status.version.as_deref(),
            "RESTORE_VERIFY_FAILED",
        )?;
        return Err(AppError::new(
            "RESTORE_VERIFY_FAILED",
            "恢复命令已执行，但主题样式节点仍然存在。",
        ));
    }
    write_log(
        app,
        "restore",
        "success",
        None,
        status.path.as_deref(),
        status.version.as_deref(),
        "OK",
    )?;
    Ok(OperationResult {
        success: true,
        action: "restore".to_string(),
        theme_id: None,
        message: "已恢复 WorkBuddy 原版，自动保持已停止。".to_string(),
        rolled_back: false,
        style_node_count: Some(0),
    })
}

fn action_for(origin: ApplyOrigin) -> &'static str {
    match origin {
        ApplyOrigin::Manual => "apply",
        ApplyOrigin::Automatic => "auto-reapply",
    }
}

fn decide_apply_mode(
    running: bool,
    cdp_available: bool,
    allow_restart: bool,
) -> AppResult<ApplyMode> {
    match (running, cdp_available, allow_restart) {
        (false, _, _) => Ok(ApplyMode::Launch),
        (true, true, _) => Ok(ApplyMode::Attach),
        (true, false, true) => Ok(ApplyMode::Restart),
        (true, false, false) => Err(AppError::new(
            "RESTART_REQUIRED",
            "WorkBuddy 正在运行但未开放 CDP。请确认重启后再应用主题。",
        )),
    }
}

fn start_monitor(
    app: AppHandle,
    operation_lock: Arc<Mutex<()>>,
    manual_operation_pending: Arc<AtomicBool>,
    monitor: Arc<MonitorRuntime>,
    background_mode: bool,
) {
    if monitor.started.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let _ = write_log(
            &app,
            "monitor",
            "started",
            None,
            None,
            None,
            "MONITOR_STARTED",
        );
        let mut retry = RetryControl::default();
        let mut ticks = 0u32;
        while !monitor.stop_requested.load(Ordering::SeqCst) {
            workbuddy_session::tick(&app);
            if ticks % 10 == 0 {
                monitor_tick(
                    &app,
                    &operation_lock,
                    &manual_operation_pending,
                    &monitor,
                    background_mode,
                    &mut retry,
                );
            }
            ticks = ticks.wrapping_add(1);
            std::thread::sleep(MONITOR_INTERVAL / 10);
        }
    });
}

fn monitor_tick(
    app: &AppHandle,
    operation_lock: &Arc<Mutex<()>>,
    manual_operation_pending: &Arc<AtomicBool>,
    monitor: &Arc<MonitorRuntime>,
    _background_mode: bool,
    retry: &mut RetryControl,
) {
    if workbuddy_session::has_trial(app) {
        return;
    }
    if monitor.retry_reset_requested.swap(false, Ordering::SeqCst) {
        retry.reset();
    }
    let saved = match load_persistent_state(app) {
        Ok(saved) => saved,
        Err(_) => {
            update_monitor(
                monitor,
                "无法读取主题状态",
                retry.attempts,
                Some("STATE_READ_FAILED"),
            );
            return;
        }
    };
    let Some(theme_id) = saved.selected_theme_id.clone() else {
        retry.reset();
        update_monitor(monitor, "自动保持已暂停", 0, None);
        return;
    };
    if saved.desired_state != DesiredState::Theme || !saved.auto_keep_theme {
        retry.reset();
        update_monitor(monitor, "自动保持已暂停", 0, None);
        return;
    }
    // Reuse this cycle's unified observation when healthy; do not open a second CDP session.
    if workbuddy_session::known_healthy(app, &theme_id) {
        retry.reset();
        update_monitor(monitor, "主题正常", 0, None);
        return;
    }
    let Some(workbuddy_path) = resolve_workbuddy_path(app).ok().flatten() else {
        update_monitor(
            monitor,
            "等待 WorkBuddy 路径",
            retry.attempts,
            Some("WORKBUDDY_NOT_FOUND"),
        );
        return;
    };
    let port = saved.preferred_port;
    if !is_workbuddy_running() {
        retry.reset_if_key_changed(format!("{}:not-running", theme_id));
        if !retry.can_attempt() {
            let (status, code) = retry.wait_status();
            update_monitor(monitor, status, retry.attempts, code);
            return;
        }
        if manual_operation_pending.load(Ordering::SeqCst) {
            return;
        }
        let Ok(lock) = operation_lock.try_lock() else {
            return;
        };
        if !auto_keep_matches(app, &theme_id) {
            drop(lock);
            return;
        }
        update_monitor(monitor, "正在启动 WorkBuddy", retry.attempts, None);
        let launched =
            launch_workbuddy(&workbuddy_path, port).and_then(|_| wait_for_renderer(port));
        drop(lock);
        match launched {
            Ok(()) => {
                retry.reset();
                update_monitor(monitor, "已连接，正在检查主题", 0, None);
            }
            Err(error) => {
                retry.record_error(&error.code);
                update_monitor(monitor, "自动应用失败", retry.attempts, Some(&error.code));
            }
        }
        return;
    }

    let target = match discover_renderer(port) {
        Ok(target) => target,
        Err(_) => {
            update_monitor(
                monitor,
                "等待 CDP；不会在当前会话强制关闭 WorkBuddy",
                retry.attempts,
                Some("CDP_UNAVAILABLE"),
            );
            return;
        }
    };
    let expected = match resolve_theme(app, &theme_id) {
        Ok(theme) => theme.record.runtime_theme_id,
        Err(error) => {
            update_monitor(monitor, "主题包不可用", retry.attempts, Some(&error.code));
            return;
        }
    };
    let metadata = read_theme_node_metadata_for_target(&target);
    let healthy = metadata.as_ref().is_ok_and(|item| {
        item.style_node_count == 1 && item.runtime_theme_id.as_deref() == Some(&expected)
    });
    let key = format!("{}:{}", theme_id, target.target_id);
    let recovery_observation_changed = retry.reset_if_key_changed(key);
    if healthy {
        retry.reset();
        update_monitor(monitor, "主题正常", 0, None);
        return;
    }
    if recovery_observation_changed {
        // Electron can expose a renderer target before its home DOM has finished mounting. Defer
        // only the first recovery attempt; later failures still use the bounded 1s/2s/5s backoff.
        retry.defer_initial_attempt(RENDERER_SETTLE_DELAY);
        update_monitor(monitor, "检测到 renderer 变化，等待页面稳定", 0, None);
        return;
    }
    if !retry.can_attempt() {
        let (status, code) = retry.wait_status();
        update_monitor(monitor, status, retry.attempts, code);
        return;
    }
    if manual_operation_pending.load(Ordering::SeqCst) {
        return;
    }
    let Ok(lock) = operation_lock.try_lock() else {
        return;
    };
    // The monitor may have observed a previous state while a manual switch was holding the
    // operation lock. Re-read after acquiring the lock so an old polling cycle cannot undo the
    // user's newly selected theme.
    if !auto_keep_matches(app, &theme_id) {
        drop(lock);
        return;
    }
    update_monitor(monitor, "主题丢失，正在自动恢复", retry.attempts, None);
    let workbuddy_version = read_workbuddy_version(&workbuddy_path);
    let _ = write_log(
        app,
        "auto-reapply",
        "attempt",
        Some(&theme_id),
        workbuddy_path.to_str(),
        workbuddy_version.as_deref(),
        "THEME_NOT_HEALTHY",
    );
    let result = apply_theme_locked(app, &theme_id, false, ApplyOrigin::Automatic);
    drop(lock);
    match result {
        Ok(_) => {
            retry.reset();
            let timestamp = now_timestamp();
            if let Ok(mut latest) = load_persistent_state(app) {
                latest.last_auto_recovery_at = Some(timestamp.clone());
                let _ = save_persistent_state(app, &latest);
            }
            update_monitor_with_time(monitor, "主题已自动恢复并验证", 0, None, Some(timestamp));
        }
        Err(error) => {
            // apply_theme_locked logs command failures; don't duplicate the same event here.
            retry.record_error(&error.code);
            let (status, code) = retry.wait_status();
            update_monitor(monitor, status, retry.attempts, code);
        }
    }
}

fn auto_keep_matches(app: &AppHandle, theme_id: &str) -> bool {
    if workbuddy_session::has_trial(app) {
        return false;
    }
    load_persistent_state(app).is_ok_and(|state| {
        state.desired_state == DesiredState::Theme
            && state.auto_keep_theme
            && state.selected_theme_id.as_deref() == Some(theme_id)
    })
}

fn update_monitor(
    monitor: &MonitorRuntime,
    status: &str,
    retry_count: u8,
    error_code: Option<&str>,
) {
    update_monitor_with_time(monitor, status, retry_count, error_code, None);
}

fn update_monitor_with_time(
    monitor: &MonitorRuntime,
    status: &str,
    retry_count: u8,
    error_code: Option<&str>,
    recovered_at: Option<String>,
) {
    if let Ok(mut snapshot) = monitor.snapshot.lock() {
        snapshot.status = status.to_string();
        snapshot.retry_count = retry_count;
        snapshot.last_error_code = error_code.map(str::to_string);
        if recovered_at.is_some() {
            snapshot.last_auto_recovery_at = recovered_at;
        }
    }
}

fn resource_root(app: &AppHandle) -> AppResult<PathBuf> {
    if let Ok(bundled) = app.path().resource_dir() {
        if bundled.join("themes.json").is_file() {
            return Ok(bundled);
        }
    }
    project_root()
}

fn project_root() -> AppResult<PathBuf> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| AppError::new("RESOURCE_DIRECTORY_FAILED", "无法定位项目资源目录。"))
}

fn runtime_root() -> AppResult<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(root) = env::var_os("STUDIO_QA_ROOT") {
        return Ok(PathBuf::from(root));
    }
    let local_data = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| AppError::new("APP_DATA_DIRECTORY_FAILED", "无法定位本地应用数据目录。"))?;
    Ok(local_data.join("WorkBuddyThemeSwitcher"))
}

fn log_directory() -> AppResult<PathBuf> {
    Ok(runtime_root()?.join("logs"))
}
fn state_path() -> AppResult<PathBuf> {
    Ok(runtime_root()?.join("state.json"))
}

fn load_persistent_state(app: &AppHandle) -> AppResult<PersistentState> {
    let path = state_path()?;
    if !path.exists() {
        let mut migrated = PersistentState::default();
        let legacy_path = project_root()?.join(".runtime").join("settings.json");
        if let Ok(raw) = fs::read_to_string(legacy_path) {
            if let Ok(legacy) = serde_json::from_str::<LegacySettings>(&raw) {
                migrated.workbuddy_path = legacy.workbuddy_path;
            }
        }
        return Ok(migrated);
    }
    let raw = fs::read_to_string(&path)
        .map_err(|_| AppError::new("STATE_READ_FAILED", "无法读取已保存的主题状态。"))?;
    let mut saved = match serde_json::from_str::<PersistentState>(&raw) {
        Ok(saved) => saved,
        // A damaged local file must not crash the app or start an unwanted injection. Safe fallback
        // is the native state with automatic keep disabled.
        Err(_) => return Ok(PersistentState::default()),
    };
    saved.preferred_port = compatible_cdp_port(saved.preferred_port);
    if saved.selected_theme_id.is_none() {
        saved.auto_keep_theme = false;
    }
    saved.schema_version = 2;
    let _ = app;
    Ok(saved)
}

fn compatible_cdp_port(port: u16) -> u16 {
    if port == 0 || port == LEGACY_CDP_PORT {
        DEFAULT_CDP_PORT
    } else {
        port
    }
}

fn save_persistent_state(app: &AppHandle, state: &PersistentState) -> AppResult<()> {
    let path = state_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| AppError::new("STATE_WRITE_FAILED", "无法定位状态目录。"))?;
    fs::create_dir_all(parent)
        .map_err(|_| AppError::new("STATE_WRITE_FAILED", "无法创建状态目录。"))?;
    let body = serde_json::to_vec_pretty(state)
        .map_err(|_| AppError::new("STATE_WRITE_FAILED", "无法序列化主题状态。"))?;
    atomic_write(&path, &body)
        .map_err(|_| AppError::new("STATE_WRITE_FAILED", "无法原子保存主题状态。"))?;
    let _ = app;
    Ok(())
}

fn atomic_write(path: &Path, body: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension(format!("json.{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut file = File::create(&temporary)?;
    file.write_all(body)?;
    file.sync_all()?;
    drop(file);
    #[cfg(windows)]
    {
        let old = wide_path(&temporary);
        let new = wide_path(path);
        // MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH: state.json is either its previous
        // complete contents or the new complete contents after an interruption.
        if unsafe { MoveFileExW(old.as_ptr(), new.as_ptr(), 0x1 | 0x8) } == 0 {
            let _ = fs::remove_file(&temporary);
            return Err(io::Error::last_os_error());
        }
    }
    #[cfg(not(windows))]
    fs::rename(temporary, path)?;
    Ok(())
}

fn load_catalog(root: &Path) -> AppResult<Vec<ThemeRecord>> {
    let raw = fs::read_to_string(root.join("themes.json"))
        .map_err(|_| AppError::new("THEME_CATALOG_FAILED", "无法读取 themes.json。"))?;
    let themes: Vec<ThemeRecord> = serde_json::from_str(&raw)
        .map_err(|_| AppError::new("THEME_CATALOG_FAILED", "themes.json 格式无效。"))?;
    if themes.is_empty()
        || themes
            .iter()
            .any(|theme| theme.id.trim().is_empty() || theme.runtime_theme_id.trim().is_empty())
    {
        return Err(AppError::new(
            "THEME_CATALOG_FAILED",
            "主题清单缺少必要字段。",
        ));
    }
    for theme in &themes {
        let package = resolve_resource_path(root, &theme.package_path)?;
        let preview = resolve_resource_path(root, &theme.preview_path)?;
        if !package.is_file() || !preview.is_file() {
            return Err(AppError::new(
                "THEME_PACKAGE_MISSING",
                "内置主题包或预览图缺失。",
            ));
        }
    }
    Ok(themes)
}

fn theme_catalog(app: &AppHandle) -> AppResult<Vec<ThemeRecord>> {
    let root = resource_root(app)?;
    let mut themes = load_catalog(&root)?;
    for theme in &mut themes {
        theme.package_path = resolve_resource_path(&root, &theme.package_path)?
            .to_string_lossy()
            .to_string();
        theme.preview_path = resolve_resource_path(&root, &theme.preview_path)?
            .to_string_lossy()
            .to_string();
        theme.is_custom = false;
    }
    themes.extend(load_custom_catalog()?);
    let store = theme_library::ThemeStore::production()?;
    for item in store.list()? {
        themes.push(store.record(&item.reference)?);
    }
    Ok(themes)
}

fn custom_theme_root() -> AppResult<PathBuf> {
    Ok(runtime_root()?.join("custom-themes"))
}

fn custom_theme_directory(theme_id: &str) -> AppResult<PathBuf> {
    if !is_safe_custom_theme_id(theme_id) {
        return Err(AppError::new(
            "CUSTOM_THEME_INVALID",
            "自定义主题标识无效。",
        ));
    }
    Ok(custom_theme_root()?.join(theme_id))
}

fn is_safe_custom_theme_id(theme_id: &str) -> bool {
    theme_id.starts_with("custom-")
        && theme_id.len() <= 80
        && theme_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn load_custom_catalog() -> AppResult<Vec<ThemeRecord>> {
    let root = custom_theme_root()?;
    if !root.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(&root)
        .map_err(|_| AppError::new("CUSTOM_THEME_READ_FAILED", "无法读取自定义主题目录。"))?;
    let mut themes = Vec::new();
    for entry in entries.flatten() {
        let directory = entry.path();
        if !directory.is_dir() {
            continue;
        }
        let metadata_path = directory.join("theme.json");
        let Ok(raw) = fs::read_to_string(&metadata_path) else {
            continue;
        };
        let Ok(mut record) = serde_json::from_str::<ThemeRecord>(&raw) else {
            continue;
        };
        if !is_safe_custom_theme_id(&record.id) || directory != custom_theme_directory(&record.id)?
        {
            continue;
        }
        record.is_custom = true;
        let package = PathBuf::from(&record.package_path);
        let preview = PathBuf::from(&record.preview_path);
        if package.parent() == Some(directory.as_path())
            && preview.parent() == Some(directory.as_path())
            && package.is_file()
            && preview.is_file()
        {
            themes.push(record);
        }
    }
    themes.sort_by(|left, right| right.id.cmp(&left.id));
    Ok(themes)
}

fn resolve_theme(app: &AppHandle, requested_id: &str) -> AppResult<ResolvedTheme> {
    if requested_id.starts_with("trial-") {
        return workbuddy_session::resolve_trial(app, requested_id);
    }
    if requested_id.starts_with("custom-v2-") {
        let reference = theme_library::ThemeRef::parse(requested_id)?;
        let record = theme_library::ThemeStore::production()?.record(&reference)?;
        return Ok(ResolvedTheme {
            package_path: PathBuf::from(&record.package_path),
            record,
        });
    }
    let record = theme_catalog(app)?
        .into_iter()
        .find(|theme| theme.id == requested_id)
        .ok_or_else(|| AppError::new("THEME_NOT_FOUND", "所选主题不存在。"))?;
    let package_path = PathBuf::from(&record.package_path);
    if !package_path.is_file() {
        return Err(AppError::new("THEME_PACKAGE_MISSING", "所选主题包不存在。"));
    }
    Ok(ResolvedTheme {
        record,
        package_path,
    })
}

fn resolve_resource_path(root: &Path, relative: &str) -> AppResult<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(AppError::new(
            "THEME_CATALOG_FAILED",
            "主题清单包含不安全的资源路径。",
        ));
    }
    Ok(root.join(path))
}

fn resolve_workbuddy_path(app: &AppHandle) -> AppResult<Option<PathBuf>> {
    let mut saved = load_persistent_state(app)?;

    // A manually chosen path remains authoritative while it is valid. If the app was moved or
    // upgraded in place, fall through to discovery and replace the stale entry below.
    if let Some(path) = saved.workbuddy_path.as_ref() {
        if validate_workbuddy_path(path).is_ok() {
            return Ok(Some(path.clone()));
        }
    }

    let discovered = workbuddy_path_candidates()
        .into_iter()
        .find(|path| validate_workbuddy_path(path).is_ok());
    if let Some(path) = discovered.as_ref() {
        if saved.workbuddy_path.as_ref() != Some(path) {
            saved.workbuddy_path = Some(path.clone());
            save_persistent_state(app, &saved)?;
        }
    }
    Ok(discovered)
}

fn workbuddy_path_candidates() -> Vec<PathBuf> {
    let mut candidates = registry_workbuddy_candidates();
    if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
        let local_app_data = PathBuf::from(local_app_data);
        candidates.push(
            local_app_data
                .join("Programs")
                .join("WorkBuddy")
                .join("WorkBuddy.exe"),
        );
        candidates.push(local_app_data.join("WorkBuddy").join("WorkBuddy.exe"));
    }
    for program_files in [
        env::var_os("PROGRAMFILES"),
        env::var_os("PROGRAMFILES(X86)"),
    ]
    .into_iter()
    .flatten()
    {
        candidates.push(
            PathBuf::from(program_files)
                .join("WorkBuddy")
                .join("WorkBuddy.exe"),
        );
    }
    candidates.extend(drive_root_workbuddy_candidates('C'..='Z'));
    deduplicate_paths(candidates)
}

fn drive_root_workbuddy_candidates(drives: impl IntoIterator<Item = char>) -> Vec<PathBuf> {
    drives
        .into_iter()
        .flat_map(|drive| {
            let root = PathBuf::from(format!("{drive}:\\"));
            [
                root.join("WorkBuddy").join("WorkBuddy.exe"),
                root.join("Program Files")
                    .join("WorkBuddy")
                    .join("WorkBuddy.exe"),
            ]
        })
        .collect()
}

fn deduplicate_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut unique = Vec::new();
    for path in paths {
        if !unique.iter().any(|known: &PathBuf| known == &path) {
            unique.push(path);
        }
    }
    unique
}

#[cfg(windows)]
fn registry_workbuddy_candidates() -> Vec<PathBuf> {
    const UNINSTALL_ROOTS: [&str; 3] = [
        r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    ];

    let mut keys = Vec::new();
    for root in UNINSTALL_ROOTS {
        if let Some(output) = run_registry_command(&["query", root, "/s", "/v", "DisplayName"]) {
            keys.extend(registry_workbuddy_install_keys(&output));
        }
    }

    let mut candidates = Vec::new();
    for key in keys {
        if let Some(location) = registry_value(&key, "InstallLocation") {
            candidates.push(PathBuf::from(location).join("WorkBuddy.exe"));
        }
        if let Some(icon) = registry_value(&key, "DisplayIcon") {
            let executable = icon
                .split(',')
                .next()
                .unwrap_or_default()
                .trim()
                .trim_matches('"');
            if !executable.is_empty() {
                candidates.push(PathBuf::from(executable));
            }
        }
    }
    candidates
}

#[cfg(not(windows))]
fn registry_workbuddy_candidates() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn run_registry_command(arguments: &[&str]) -> Option<String> {
    let mut command = Command::new("reg.exe");
    configure_hidden_command(&mut command);
    command.args(arguments);
    let output = output_with_deadline(&mut command, Duration::from_secs(8)).ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(windows)]
fn registry_workbuddy_install_keys(output: &str) -> Vec<String> {
    let mut current_key = None;
    let mut matches = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("HKEY_") {
            current_key = Some(trimmed.to_string());
            continue;
        }
        if registry_value_from_line(trimmed, "DisplayName")
            .is_some_and(|name| name.to_ascii_lowercase().contains("workbuddy"))
        {
            if let Some(key) = current_key.as_ref() {
                matches.push(key.clone());
            }
        }
    }
    matches
}

#[cfg(windows)]
fn registry_value(key: &str, value_name: &str) -> Option<String> {
    let output = run_registry_command(&["query", key, "/v", value_name])?;
    output
        .lines()
        .find_map(|line| registry_value_from_line(line.trim(), value_name))
}

#[cfg(windows)]
fn registry_value_from_line(line: &str, value_name: &str) -> Option<String> {
    if !line
        .to_ascii_lowercase()
        .starts_with(&value_name.to_ascii_lowercase())
    {
        return None;
    }
    for kind in ["REG_SZ", "REG_EXPAND_SZ"] {
        if let Some(index) = line.find(kind) {
            let value = line[index + kind.len()..].trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn validate_workbuddy_path(path: &Path) -> AppResult<()> {
    let filename_matches = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("WorkBuddy.exe"));
    if !filename_matches || !path.is_file() {
        return Err(AppError::new(
            "WORKBUDDY_PATH_INVALID",
            "请选择有效的 WorkBuddy.exe。",
        ));
    }
    let app_asar = path
        .parent()
        .map(|parent| parent.join("resources").join("app.asar"));
    if !app_asar.is_some_and(|file| file.is_file()) {
        return Err(AppError::new(
            "WORKBUDDY_PATH_INVALID",
            "所选 WorkBuddy 缺少 resources\\app.asar。",
        ));
    }
    Ok(())
}

#[repr(C)]
struct VsFixedFileInfo {
    signature: u32,
    struct_version: u32,
    file_version_ms: u32,
    file_version_ls: u32,
    product_version_ms: u32,
    product_version_ls: u32,
    file_flags_mask: u32,
    file_flags: u32,
    file_os: u32,
    file_type: u32,
    file_subtype: u32,
    file_date_ms: u32,
    file_date_ls: u32,
}

#[cfg(windows)]
#[link(name = "Version")]
extern "system" {
    fn GetFileVersionInfoSizeW(filename: *const u16, handle: *mut u32) -> u32;
    fn GetFileVersionInfoW(
        filename: *const u16,
        handle: u32,
        length: u32,
        data: *mut c_void,
    ) -> i32;
    fn VerQueryValueW(
        block: *const c_void,
        sub_block: *const u16,
        buffer: *mut *mut c_void,
        length: *mut u32,
    ) -> i32;
}

#[cfg(windows)]
#[link(name = "Kernel32")]
extern "system" {
    fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
}

#[cfg(windows)]
#[link(name = "Advapi32")]
extern "system" {
    fn CredDeleteW(target_name: *const u16, credential_type: u32, flags: u32) -> i32;
}

#[cfg(windows)]
#[link(name = "User32")]
extern "system" {
    fn EnumWindows(
        callback: Option<unsafe extern "system" fn(isize, isize) -> i32>,
        lparam: isize,
    ) -> i32;
    fn GetWindowThreadProcessId(window: isize, process_id: *mut u32) -> u32;
    fn IsWindowVisible(window: isize) -> i32;
    fn PostMessageW(window: isize, message: u32, wparam: usize, lparam: isize) -> i32;
}

#[cfg(windows)]
fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn wide_string(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn read_workbuddy_version(path: &Path) -> Option<String> {
    #[cfg(windows)]
    unsafe {
        let file = wide_path(path);
        let mut handle = 0u32;
        let size = GetFileVersionInfoSizeW(file.as_ptr(), &mut handle);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(file.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
            return None;
        }
        let mut value: *mut c_void = std::ptr::null_mut();
        let mut value_size = 0u32;
        let root = [b'\\' as u16, 0];
        if VerQueryValueW(
            data.as_ptr().cast(),
            root.as_ptr(),
            &mut value,
            &mut value_size,
        ) == 0
            || value_size < std::mem::size_of::<VsFixedFileInfo>() as u32
        {
            return None;
        }
        let info = &*(value as *const VsFixedFileInfo);
        if info.signature != 0xfeef04bd {
            return None;
        }
        Some(format!(
            "{}.{}.{}.{}",
            info.product_version_ms >> 16,
            info.product_version_ms & 0xffff,
            info.product_version_ls >> 16,
            info.product_version_ls & 0xffff
        ))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

fn is_supported_workbuddy_version(version: Option<&str>) -> bool {
    let Some(version) = version else {
        // The renderer probe remains the final compatibility check when Windows cannot expose
        // file-version metadata, so an unknown version must not block a valid installation.
        return true;
    };
    let mut parts = version.split('.').map(|part| part.parse::<u16>());
    let (Some(Ok(major)), Some(Ok(minor)), Some(Ok(patch))) =
        (parts.next(), parts.next(), parts.next())
    else {
        return true;
    };
    major == 5
        && (minor, patch)
            >= (
                MIN_SUPPORTED_WORKBUDDY_VERSION.1,
                MIN_SUPPORTED_WORKBUDDY_VERSION.2,
            )
}

fn ensure_supported_workbuddy_version(version: Option<&str>) -> AppResult<()> {
    if is_supported_workbuddy_version(version) {
        return Ok(());
    }
    Err(AppError::new(
        "WORKBUDDY_VERSION_UNSUPPORTED",
        format!(
            "当前 WorkBuddy {} 不在兼容范围内；主题切换器支持 5.2.6 至 5.x。",
            version.unwrap_or("未知版本")
        ),
    ))
}

fn configure_hidden_command(command: &mut Command) {
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let _ = command;
}

fn hidden_output(program: &str, args: &[&str]) -> io::Result<std::process::Output> {
    let mut command = Command::new(program);
    configure_hidden_command(&mut command);
    command.args(args);
    output_with_deadline(&mut command, Duration::from_secs(8))
}

fn output_with_deadline(command: &mut Command, timeout: Duration) -> io::Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let capture = |reader: Box<dyn Read + Send>| {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = reader
                .take(4 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = sender.send(result);
        });
        receiver
    };
    let stdout = capture(Box::new(child.stdout.take().expect("piped stdout")));
    let stderr = capture(Box::new(child.stderr.take().expect("piped stderr")));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "child task deadline exceeded",
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let stdout = stdout
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "stdout deadline exceeded"))??;
    let stderr = stderr
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "stderr deadline exceeded"))??;
    if stdout.len() > 4 * 1024 * 1024 || stderr.len() > 4 * 1024 * 1024 {
        return Err(io::Error::other("child output limit exceeded"));
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn workbuddy_pids() -> Vec<u32> {
    hidden_output(
        "tasklist.exe",
        &["/FI", "IMAGENAME eq WorkBuddy.exe", "/FO", "CSV", "/NH"],
    )
    .ok()
    .and_then(|output| String::from_utf8(output.stdout).ok())
    .map(|rows| {
        rows.lines()
            .filter_map(|line| {
                let mut cells = line.trim_matches('"').split("\",\"");
                let image = cells.next()?;
                let pid = cells.next()?;
                image
                    .eq_ignore_ascii_case("WorkBuddy.exe")
                    .then(|| pid.parse::<u32>().ok())
                    .flatten()
            })
            .collect()
    })
    .unwrap_or_default()
}

fn is_workbuddy_running() -> bool {
    !workbuddy_pids().is_empty()
}

#[cfg(windows)]
unsafe extern "system" fn close_workbuddy_window_callback(window: isize, lparam: isize) -> i32 {
    let pids = &*(lparam as *const Vec<u32>);
    let mut pid = 0u32;
    GetWindowThreadProcessId(window, &mut pid);
    if pids.contains(&pid) && IsWindowVisible(window) != 0 {
        const WM_CLOSE: u32 = 0x0010;
        let _ = PostMessageW(window, WM_CLOSE, 0, 0);
    }
    1
}

fn request_graceful_workbuddy_close() -> AppResult<()> {
    let pids = workbuddy_pids();
    if pids.is_empty() {
        return Ok(());
    }
    #[cfg(windows)]
    unsafe {
        let _ = EnumWindows(
            Some(close_workbuddy_window_callback),
            &pids as *const Vec<u32> as isize,
        );
    }
    for _ in 0..32 {
        if !is_workbuddy_running() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(AppError::new(
        "GRACEFUL_CLOSE_PENDING",
        "WorkBuddy 尚未关闭；请先保存工作并手动关闭，然后再次应用主题。",
    ))
}

fn launch_workbuddy(path: &Path, port: u16) -> AppResult<()> {
    validate_workbuddy_path(path)?;
    let mut command = Command::new(path);
    configure_hidden_command(&mut command);
    command
        .arg("--remote-debugging-address=127.0.0.1")
        .arg(format!("--remote-debugging-port={port}"))
        .env_remove("ELECTRON_RUN_AS_NODE")
        .env("WORKBUDDY_REMOTE_DEBUGGING_PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| {
            AppError::new(
                "WORKBUDDY_LAUNCH_FAILED",
                "无法以本地 CDP 模式启动 WorkBuddy。",
            )
        })?;
    Ok(())
}

fn wait_for_renderer(port: u16) -> AppResult<()> {
    for _ in 0..60 {
        if discover_renderer(port).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(AppError::new(
        "CDP_START_TIMEOUT",
        "等待 WorkBuddy renderer 超时。",
    ))
}

fn renderer_matches(target: &CdpTarget) -> bool {
    target.target_type == "page"
        && target.websocket_debugger_url.is_some()
        && target
            .url
            .replace('\\', "/")
            .to_ascii_lowercase()
            .contains(RENDERER_PATH)
        && (target.title.to_ascii_lowercase().contains("workbuddy")
            || target
                .description
                .as_deref()
                .is_some_and(|value| value.to_ascii_lowercase().contains("workbuddy"))
            || target.url.to_ascii_lowercase().contains("workbuddy"))
}

fn discover_renderer(port: u16) -> AppResult<RendererTarget> {
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .try_proxy_from_env(false)
        .timeout(CDP_STATUS_PROBE_TIMEOUT)
        .build();
    let url = format!("http://127.0.0.1:{port}/json/list");
    let response = agent
        .get(&url)
        .call()
        .map_err(|_| AppError::new("CDP_UNAVAILABLE", "WorkBuddy 的本地 CDP 未连接。"))?;
    let targets: Vec<CdpTarget> = response
        .into_json()
        .map_err(|_| AppError::new("CDP_PROTOCOL_ERROR", "WorkBuddy CDP 返回了无效数据。"))?;
    let target = targets
        .into_iter()
        .find(renderer_matches)
        .ok_or_else(|| AppError::new("CDP_RENDERER_NOT_FOUND", "未找到 WorkBuddy renderer。"))?;
    Ok(RendererTarget {
        target_id: target.id,
        websocket_debugger_url: target.websocket_debugger_url.unwrap_or_default(),
    })
}

fn read_theme_node_metadata(port: u16) -> AppResult<ThemeNodeMetadata> {
    let target = discover_renderer(port)?;
    read_theme_node_metadata_for_target(&target)
}

fn read_theme_node_metadata_for_target(target: &RendererTarget) -> AppResult<ThemeNodeMetadata> {
    let endpoint = url::Url::parse(&target.websocket_debugger_url)
        .map_err(|_| AppError::new("CDP_UNAVAILABLE", "CDP 地址无效。"))?;
    if endpoint.scheme() != "ws"
        || !matches!(endpoint.host_str(), Some("localhost" | "127.0.0.1"))
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
    {
        return Err(AppError::new("CDP_UNAVAILABLE", "CDP 必须绑定本机。"));
    }
    let address = format!("127.0.0.1:{}", endpoint.port().unwrap_or(DEFAULT_CDP_PORT))
        .parse()
        .map_err(|_| AppError::new("CDP_UNAVAILABLE", "CDP 端口无效。"))?;
    let stream = TcpStream::connect_timeout(&address, CDP_STATUS_PROBE_TIMEOUT)
        .map_err(|_| AppError::new("CDP_UNAVAILABLE", "CDP 连接超时。"))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(750)))
        .map_err(|_| AppError::new("CDP_UNAVAILABLE", "无法设置 CDP 超时。"))?;
    stream
        .set_write_timeout(Some(Duration::from_millis(750)))
        .map_err(|_| AppError::new("CDP_UNAVAILABLE", "无法设置 CDP 超时。"))?;
    let (mut socket, _) = client(
        target.websocket_debugger_url.as_str(),
        MaybeTlsStream::Plain(stream),
    )
    .map_err(|_| AppError::new("CDP_UNAVAILABLE", "无法连接 WorkBuddy renderer。"))?;
    set_socket_timeout(&mut socket)?;
    // This only returns node count and CodeDrobe's own theme identifier. It never reads DOM text,
    // user input, account details, or message content.
    let expression = format!("(() => ({{ styleNodeCount: document.querySelectorAll('#{}').length, runtimeThemeId: document.documentElement?.dataset?.codedrobeTheme ?? null }}))()", CODEDROBE_STYLE_ID);
    socket.send(Message::Text(json!({ "id": 1, "method": "Runtime.evaluate", "params": { "expression": expression, "returnByValue": true } }).to_string().into()))
        .map_err(|_| AppError::new("CDP_UNAVAILABLE", "无法向 WorkBuddy renderer 发送检查请求。"))?;
    let started = Instant::now();
    loop {
        if started.elapsed() > Duration::from_secs(2) {
            return Err(AppError::new("CDP_UNAVAILABLE", "CDP 检查超过总时间限制。"));
        }
        match socket
            .read()
            .map_err(|_| AppError::new("CDP_UNAVAILABLE", "WorkBuddy renderer 在检查期间断开。"))?
        {
            Message::Text(text) => {
                let response: Value = serde_json::from_str(&text).map_err(|_| {
                    AppError::new("CDP_PROTOCOL_ERROR", "WorkBuddy CDP 返回了无效数据。")
                })?;
                if response.get("id").and_then(Value::as_u64) != Some(1) {
                    continue;
                }
                if response.get("error").is_some()
                    || response.pointer("/result/exceptionDetails").is_some()
                {
                    return Err(AppError::new(
                        "CDP_PROTOCOL_ERROR",
                        "WorkBuddy renderer 拒绝了状态检查。",
                    ));
                }
                let metadata = parse_theme_node_metadata(
                    response.pointer("/result/result/value").ok_or_else(|| {
                        AppError::new("CDP_PROTOCOL_ERROR", "WorkBuddy CDP 未返回主题状态。")
                    })?,
                );
                // Finish the short metadata session before CodeDrobe opens its own CDP session
                // to apply a replacement style. Dropping the TCP stream alone can leave Electron
                // processing the old socket briefly during a renderer refresh.
                let _ = socket.close(None);
                return metadata;
            }
            Message::Ping(payload) => socket.send(Message::Pong(payload)).map_err(|_| {
                AppError::new("CDP_UNAVAILABLE", "WorkBuddy renderer 在检查期间断开。")
            })?,
            Message::Close(_) => {
                return Err(AppError::new(
                    "CDP_UNAVAILABLE",
                    "WorkBuddy renderer 已关闭。",
                ))
            }
            Message::Binary(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }
}

fn set_socket_timeout(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>) -> AppResult<()> {
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => {
            stream
                .set_read_timeout(Some(Duration::from_millis(750)))
                .map_err(|_| AppError::new("CDP_UNAVAILABLE", "无法设置 WorkBuddy CDP 超时。"))?;
            stream
                .set_write_timeout(Some(Duration::from_millis(750)))
                .map_err(|_| AppError::new("CDP_UNAVAILABLE", "无法设置 WorkBuddy CDP 超时。"))?;
        }
        #[allow(unreachable_patterns)]
        _ => {}
    }
    Ok(())
}

fn parse_theme_node_metadata(value: &Value) -> AppResult<ThemeNodeMetadata> {
    let style_node_count = value
        .get("styleNodeCount")
        .and_then(Value::as_u64)
        .and_then(|item| u32::try_from(item).ok())
        .ok_or_else(|| AppError::new("CDP_PROTOCOL_ERROR", "主题节点数量格式无效。"))?;
    Ok(ThemeNodeMetadata {
        style_node_count,
        runtime_theme_id: value
            .get("runtimeThemeId")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn run_codedrobe(app: &AppHandle, args: &[String]) -> AppResult<Value> {
    let cli = child_process_path(&find_codedrobe_cli(app)?);
    let node = child_process_path(&find_node_runtime(app)?);
    let mut command = Command::new(node);
    configure_hidden_command(&mut command);
    command
        .arg(&cli)
        .args(args)
        .current_dir(
            cli.parent()
                .and_then(Path::parent)
                .unwrap_or_else(|| Path::new(".")),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = output_with_deadline(&mut command, Duration::from_secs(30)).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            AppError::new("NODE_NOT_FOUND", "内置 Node.js 运行时不可用。")
        } else if error.kind() == io::ErrorKind::TimedOut {
            let _ = write_log(
                app,
                "codedrobe",
                "process-timeout",
                None,
                None,
                None,
                "CODEDROBE_TIMEOUT",
            );
            AppError::new(
                "CODEDROBE_TIMEOUT",
                codedrobe_diagnostic::message("CODEDROBE_TIMEOUT"),
            )
        } else {
            AppError::new("CODEDROBE_COMMAND_FAILED", "无法启动本地 CodeDrobe Core。")
        }
    })?;
    if !output.status.success() {
        let report = codedrobe_diagnostic::parse(&String::from_utf8_lossy(&output.stderr))
            .unwrap_or(codedrobe_diagnostic::Diagnostic {
                code: "CODEDROBE_COMMAND_FAILED",
                checks: vec![],
            });
        let code = report.code;
        let diagnostic = report.log_code(output.status.code().unwrap_or(-1));
        // Keep only a static error code and exit status; command output can include target details
        // and is intentionally never written to disk.
        let _ = write_log(
            app,
            "codedrobe",
            "process-failed",
            None,
            None,
            None,
            &diagnostic,
        );
        let mut message = codedrobe_diagnostic::message(code).to_string();
        if !report.checks.is_empty() {
            message.push_str(&format!(" 检查项：{}。", report.checks.join("、")));
        }
        return Err(AppError::new(code, message));
    }
    serde_json::from_slice(&output.stdout).map_err(|_| {
        AppError::new(
            "CODEDROBE_PROTOCOL_ERROR",
            "本地 CodeDrobe Core 返回了无效结果。",
        )
    })
}

fn find_codedrobe_cli(app: &AppHandle) -> AppResult<PathBuf> {
    let root = resource_root(app)?;
    bundled_runtime_candidates(&root, "codedrobe/bin/codedrobe.mjs")
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            AppError::new(
                "CODEDROBE_NOT_FOUND",
                "内置 CodeDrobe Core 不完整。请重新安装主题切换器。",
            )
        })
}

fn find_node_runtime(app: &AppHandle) -> AppResult<PathBuf> {
    let root = resource_root(app)?;
    bundled_runtime_candidates(&root, "node/node.exe")
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            AppError::new(
                "NODE_NOT_FOUND",
                "内置 Node.js 运行时不完整。请重新安装主题切换器。",
            )
        })
}

fn bundled_runtime_candidates(root: &Path, runtime_relative_path: &str) -> Vec<PathBuf> {
    let relative = Path::new(runtime_relative_path);
    vec![root.join(relative), root.join("vendor").join(relative)]
}

/// Tauri can expose Windows resources through the extended-length `\\?\` namespace. Rust can
/// open those paths, but the embedded Node runtime cannot use such a path as its entry module.
/// Theme packages and the bundled Core are deliberately shipped well below MAX_PATH, so use the
/// normal Win32 spelling only for child-process arguments.
fn child_process_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let raw = path.to_string_lossy();
        if let Some(unc) = raw.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{unc}"));
        }
        if let Some(normal) = raw.strip_prefix(r"\\?\") {
            return PathBuf::from(normal);
        }
    }
    path.to_path_buf()
}

fn child_process_path_string(path: &Path) -> String {
    child_process_path(path).to_string_lossy().to_string()
}

fn wait_for_theme_probe(app: &AppHandle, theme: &ResolvedTheme, port: u16) -> AppResult<()> {
    run_codedrobe(app, &theme_probe_args(&theme.package_path, port)).map(|_| ())
}

fn theme_probe_args(theme_package: &Path, port: u16) -> Vec<String> {
    vec![
        "probe".to_string(),
        "--app".to_string(),
        "workbuddy".to_string(),
        "--theme".to_string(),
        child_process_path_string(theme_package),
        "--port".to_string(),
        port.to_string(),
        "--timeout-ms".to_string(),
        "10000".to_string(),
        "--json".to_string(),
    ]
}

fn codedrobe_verify_passes(value: &Value) -> bool {
    let Some(targets) = value.get("targets").and_then(Value::as_array) else {
        return false;
    };
    !targets.is_empty()
        && targets.iter().any(|target| {
            target
                .pointer("/result/pass")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        && targets.iter().all(|target| {
            target
                .get("skipped")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || target
                    .pointer("/result/pass")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
        })
}

fn restore_with_codedrobe(app: &AppHandle, port: u16) -> AppResult<Value> {
    run_codedrobe(
        app,
        &[
            "restore".to_string(),
            "--app".to_string(),
            "workbuddy".to_string(),
            "--port".to_string(),
            port.to_string(),
            "--json".to_string(),
        ],
    )
}

fn validate_controls(controls: &ThemeControls) -> AppResult<()> {
    if !(-40..=40).contains(&controls.brightness)
        || controls.blur > 24
        || !(55..=96).contains(&controls.panel_opacity)
    {
        return Err(AppError::new(
            "CUSTOM_THEME_CONTROLS_INVALID",
            "亮度、模糊和透明度超出允许范围。",
        ));
    }
    if let Some(accent) = controls.accent.as_deref() {
        if parse_hex_color(accent).is_none() {
            return Err(AppError::new(
                "CUSTOM_THEME_ACCENT_INVALID",
                "强调色必须是 #RRGGBB 格式。",
            ));
        }
    }
    Ok(())
}

fn source_image_extension(path: &Path, bytes: &[u8]) -> AppResult<&'static str> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let detected = if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        "png"
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "webp"
    } else {
        return Err(AppError::new(
            "CUSTOM_IMAGE_TYPE_INVALID",
            "仅支持 JPG、PNG 或 WebP 图片。",
        ));
    };
    let extension_matches = matches!(
        (extension.as_str(), detected),
        ("jpg" | "jpeg", "jpg") | ("png", "png") | ("webp", "webp")
    );
    if !extension_matches {
        return Err(AppError::new(
            "CUSTOM_IMAGE_TYPE_INVALID",
            "图片扩展名与实际格式不匹配。",
        ));
    }
    Ok(detected)
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[(((first & 0b0000_0011) << 4) | (second >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[(((second & 0b0000_1111) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(third & 0b0011_1111) as usize] as char
        } else {
            '='
        });
    }
    output
}

fn decode_image_base64(
    value: &str,
    max_bytes: usize,
    size_code: &'static str,
    size_message: &'static str,
) -> AppResult<Vec<u8>> {
    let max_base64_length = max_bytes.div_ceil(3) * 4;
    if value.is_empty() || value.len() > max_base64_length {
        return Err(AppError::new(size_code, size_message));
    }
    let bytes = base64_decode(value)
        .ok_or_else(|| AppError::new("CUSTOM_IMAGE_DATA_INVALID", "图片数据编码无效。"))?;
    if bytes.is_empty() || bytes.len() > max_bytes {
        return Err(AppError::new(size_code, size_message));
    }
    Ok(bytes)
}

fn base64_decode(value: &str) -> Option<Vec<u8>> {
    if value.len() % 4 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(value.len() / 4 * 3);
    for (index, chunk) in value.as_bytes().chunks_exact(4).enumerate() {
        let first = base64_value(chunk[0])?;
        let second = base64_value(chunk[1])?;
        let third_padding = chunk[2] == b'=';
        let fourth_padding = chunk[3] == b'=';
        if third_padding && !fourth_padding {
            return None;
        }
        if (third_padding || fourth_padding) && index + 1 != value.len() / 4 {
            return None;
        }
        let third = if third_padding {
            0
        } else {
            base64_value(chunk[2])?
        };
        let fourth = if fourth_padding {
            0
        } else {
            base64_value(chunk[3])?
        };
        if (third_padding && second & 0b0000_1111 != 0)
            || (fourth_padding && !third_padding && third & 0b0000_0011 != 0)
        {
            return None;
        }
        bytes.push((first << 2) | (second >> 4));
        if !third_padding {
            bytes.push((second << 4) | (third >> 2));
        }
        if !fourth_padding {
            bytes.push((third << 6) | fourth);
        }
    }
    Some(bytes)
}

fn base64_value(value: u8) -> Option<u8> {
    match value {
        b'A'..=b'Z' => Some(value - b'A'),
        b'a'..=b'z' => Some(value - b'a' + 26),
        b'0'..=b'9' => Some(value - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Explicit migration action only. Never read, enumerate or return a credential.
#[tauri::command]
fn clear_legacy_ai_credentials(confirmed: bool) -> AppResult<()> {
    if !confirmed {
        return Err(AppError::new(
            "CONFIRMATION_REQUIRED",
            "请先确认清除旧凭据。",
        ));
    }
    #[cfg(windows)]
    {
        let target = wide_string("WorkBuddyThemeSwitcher:CustomThemeAiKey");
        if unsafe { CredDeleteW(target.as_ptr(), 1, 0) } != 0
            || io::Error::last_os_error().raw_os_error() == Some(1168)
        {
            return Ok(());
        }
        Err(AppError::new(
            "CREDENTIAL_CLEAR_FAILED",
            "无法清除旧凭据；离线编辑不受影响。",
        ))
    }
    #[cfg(not(windows))]
    Err(AppError::new(
        "CREDENTIAL_UNSUPPORTED",
        "仅 Windows 支持清除历史凭据。",
    ))
}

impl RgbColor {
    fn css(self) -> String {
        format!("rgb({} {} {})", self.red, self.green, self.blue)
    }
    fn hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.red, self.green, self.blue)
    }
}

fn css_rgba(color: RgbColor, alpha: f32) -> String {
    format!(
        "rgb({} {} {} / {:.2})",
        color.red,
        color.green,
        color.blue,
        alpha.clamp(0.0, 1.0)
    )
}

fn parse_hex_color(value: &str) -> Option<RgbColor> {
    let hex = value.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(RgbColor {
        red: u8::from_str_radix(&hex[0..2], 16).ok()?,
        green: u8::from_str_radix(&hex[2..4], 16).ok()?,
        blue: u8::from_str_radix(&hex[4..6], 16).ok()?,
    })
}

fn mix(left: RgbColor, right: RgbColor, amount: f32) -> RgbColor {
    let amount = amount.clamp(0.0, 1.0);
    let blend =
        |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * amount).round() as u8;
    RgbColor {
        red: blend(left.red, right.red),
        green: blend(left.green, right.green),
        blue: blend(left.blue, right.blue),
    }
}
fn adjust_brightness(color: RgbColor, adjustment: i16) -> RgbColor {
    let shift = |channel: u8| (i16::from(channel) + adjustment).clamp(0, 255) as u8;
    RgbColor {
        red: shift(color.red),
        green: shift(color.green),
        blue: shift(color.blue),
    }
}
fn relative_luminance(color: RgbColor) -> f32 {
    let convert = |channel: u8| {
        let value = f32::from(channel) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * convert(color.red) + 0.7152 * convert(color.green) + 0.0722 * convert(color.blue)
}
fn contrast_ratio(left: RgbColor, right: RgbColor) -> f32 {
    let (bright, dark) = (relative_luminance(left), relative_luminance(right));
    (bright.max(dark) + 0.05) / (bright.min(dark) + 0.05)
}

fn persist_manual_theme_choice(app: &AppHandle, theme_id: &str) -> AppResult<()> {
    let theme = resolve_theme(app, theme_id)?;
    let mut saved = load_persistent_state(app)?;
    if saved.workbuddy_path.is_none() {
        saved.workbuddy_path = resolve_workbuddy_path(app)?;
    }
    saved.desired_state = DesiredState::Theme;
    saved.selected_theme_id = Some(theme.record.id);
    saved.selected_theme_version = Some(theme.record.theme_version);
    saved.auto_keep_theme = true;
    saved.last_successful_apply_at = Some(now_timestamp());
    saved.last_successful_verify_theme = saved.selected_theme_id.clone();
    save_persistent_state(app, &saved)?;
    set_login_autostart(app, true)
}

fn persist_original_choice(app: &AppHandle) -> AppResult<()> {
    let mut saved = load_persistent_state(app)?;
    saved.desired_state = DesiredState::Original;
    saved.auto_keep_theme = false;
    saved.selected_theme_id = None;
    saved.selected_theme_version = None;
    save_persistent_state(app, &saved)
}

fn set_login_autostart(app: &AppHandle, enabled: bool) -> AppResult<()> {
    #[cfg(debug_assertions)]
    if env::var_os("STUDIO_QA_ROOT").is_some() {
        return Ok(());
    }
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|_| {
        AppError::new(
            "AUTOSTART_UPDATE_FAILED",
            "无法更新当前用户的开机自动保持设置。",
        )
    })
}

fn login_autostart_enabled(app: &AppHandle) -> bool {
    #[cfg(debug_assertions)]
    if env::var_os("STUDIO_QA_ROOT").is_some() {
        return false;
    }
    app.autolaunch().is_enabled().unwrap_or(false)
}

fn now_timestamp() -> String {
    Local::now().to_rfc3339()
}

fn write_log(
    app: &AppHandle,
    action: &str,
    outcome: &str,
    theme_id: Option<&str>,
    workbuddy_path: Option<&str>,
    version: Option<&str>,
    code: &str,
) -> AppResult<()> {
    let directory = log_directory()?;
    fs::create_dir_all(&directory)
        .map_err(|_| AppError::new("LOG_WRITE_FAILED", "无法创建日志目录。"))?;
    let filename = directory.join(format!(
        "theme-switcher-{}.log",
        Local::now().format("%Y-%m-%d")
    ));
    let line = format!(
        "{} action={} outcome={} theme={} workbuddy_path={} workbuddy_version={} code={}\n",
        Local::now().format("%Y-%m-%d %H:%M:%S"),
        safe_log_value(action),
        safe_log_value(outcome),
        safe_log_value(theme_id.unwrap_or("none")),
        safe_log_value(workbuddy_path.unwrap_or("unknown")),
        safe_log_value(version.unwrap_or("unknown")),
        safe_log_value(code),
    );
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(filename)
        .map_err(|_| AppError::new("LOG_WRITE_FAILED", "无法写入主题操作日志。"))?;
    file.write_all(line.as_bytes())
        .map_err(|_| AppError::new("LOG_WRITE_FAILED", "无法写入主题操作日志。"))?;
    let _ = app;
    Ok(())
}

fn safe_log_value(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn historical_credential_cleanup_requires_explicit_consent() {
        assert_eq!(
            clear_legacy_ai_credentials(false).unwrap_err().code,
            "CONFIRMATION_REQUIRED"
        );
    }

    #[test]
    fn custom_theme_preview_scope_uses_a_supported_tauri_path_variable() {
        let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let scopes = config
            .pointer("/app/security/assetProtocol/scope")
            .and_then(Value::as_array)
            .unwrap();

        assert!(scopes.iter().any(|scope| {
            scope.as_str() == Some("$LOCALDATA/WorkBuddyThemeSwitcher/custom-themes/**")
        }));
        assert!(!scopes.iter().any(|scope| {
            scope
                .as_str()
                .is_some_and(|scope| scope.contains("$LOCALAPPDATA"))
        }));
    }

    #[test]
    fn theme_resource_paths_must_be_relative() {
        let root = Path::new(r"D:\themes");
        assert!(resolve_resource_path(root, "themes/ice-blue/theme.codedrobe-theme").is_ok());
        assert!(resolve_resource_path(root, "../outside.theme").is_err());
        assert!(resolve_resource_path(root, r"D:\outside.theme").is_err());
    }

    #[test]
    fn bundled_runtime_candidates_cover_packaged_and_development_layouts() {
        let root = Path::new(r"D:\theme-switcher");
        assert_eq!(
            bundled_runtime_candidates(root, "node/node.exe"),
            vec![
                root.join("node").join("node.exe"),
                root.join("vendor").join("node").join("node.exe"),
            ]
        );
        assert_eq!(
            bundled_runtime_candidates(root, "codedrobe/bin/codedrobe.mjs"),
            vec![
                root.join("codedrobe").join("bin").join("codedrobe.mjs"),
                root.join("vendor")
                    .join("codedrobe")
                    .join("bin")
                    .join("codedrobe.mjs"),
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn child_process_paths_strip_the_windows_extended_length_prefix() {
        assert_eq!(
            child_process_path(Path::new(r"\\?\D:\switcher\codedrobe.mjs")),
            PathBuf::from(r"D:\switcher\codedrobe.mjs")
        );
        assert_eq!(
            child_process_path(Path::new(r"\\?\UNC\server\share\theme.codedrobe-theme")),
            PathBuf::from(r"\\server\share\theme.codedrobe-theme")
        );
    }

    #[test]
    fn theme_probe_waits_for_renderer_compatibility_before_apply() {
        let package = Path::new(r"D:\themes\sky-breeze\theme.codedrobe-theme");
        assert_eq!(
            theme_probe_args(package, 9336),
            vec![
                "probe".to_string(),
                "--app".to_string(),
                "workbuddy".to_string(),
                "--theme".to_string(),
                package.to_string_lossy().to_string(),
                "--port".to_string(),
                "9336".to_string(),
                "--timeout-ms".to_string(),
                "10000".to_string(),
                "--json".to_string(),
            ]
        );
    }

    #[test]
    fn apply_mode_requires_explicit_restart_confirmation() {
        assert_eq!(
            decide_apply_mode(false, false, false).unwrap(),
            ApplyMode::Launch
        );
        assert_eq!(
            decide_apply_mode(true, true, false).unwrap(),
            ApplyMode::Attach
        );
        assert!(decide_apply_mode(true, false, false).is_err());
        assert_eq!(
            decide_apply_mode(true, false, true).unwrap(),
            ApplyMode::Restart
        );
    }

    #[test]
    fn renderer_match_rejects_unrelated_cdp_pages() {
        let target = CdpTarget {
            id: "target-1".to_string(),
            target_type: "page".to_string(),
            title: "WorkBuddy".to_string(),
            url: "file:///D:/workbuddy/resources/app.asar/renderer/index.html".to_string(),
            websocket_debugger_url: Some("ws://127.0.0.1/devtools/page/1".to_string()),
            description: None,
        };
        assert!(renderer_matches(&target));
        assert!(!renderer_matches(&CdpTarget {
            url: "https://example.test".to_string(),
            ..target
        }));
    }

    #[test]
    fn metadata_parser_accepts_only_a_numeric_count() {
        let value = json!({ "styleNodeCount": 1, "runtimeThemeId": "workbuddy-ice-blue" });
        let metadata = parse_theme_node_metadata(&value).unwrap();
        assert_eq!(metadata.style_node_count, 1);
        assert_eq!(
            metadata.runtime_theme_id.as_deref(),
            Some("workbuddy-ice-blue")
        );
        assert!(parse_theme_node_metadata(&json!({ "styleNodeCount": "one" })).is_err());
    }

    #[test]
    fn verify_result_requires_every_active_target_to_pass() {
        let passing = json!({ "targets": [{ "result": { "pass": true } }] });
        let failed =
            json!({ "targets": [{ "result": { "pass": true } }, { "result": { "pass": false } }] });
        let skipped = json!({ "targets": [{ "result": { "pass": true } }, { "skipped": true }] });
        assert!(codedrobe_verify_passes(&passing));
        assert!(!codedrobe_verify_passes(&failed));
        assert!(codedrobe_verify_passes(&skipped));
    }

    #[test]
    fn old_state_without_runtime_fields_is_compatible() {
        let state: PersistentState =
            serde_json::from_str(r#"{ "workbuddyPath": "D:\\workbuddy\\WorkBuddy.exe" }"#).unwrap();
        assert_eq!(state.desired_state, DesiredState::Original);
        assert!(!state.auto_keep_theme);
        assert_eq!(state.preferred_port, DEFAULT_CDP_PORT);
    }

    #[test]
    fn legacy_workbuddy_cdp_port_migrates_to_the_compatible_default() {
        assert_eq!(compatible_cdp_port(0), DEFAULT_CDP_PORT);
        assert_eq!(compatible_cdp_port(LEGACY_CDP_PORT), DEFAULT_CDP_PORT);
        assert_eq!(compatible_cdp_port(24567), 24567);
    }

    #[test]
    fn retry_limits_to_three_and_resets_for_a_new_target() {
        let mut retry = RetryControl::default();
        retry.reset_if_key_changed("ice:one".to_string());
        for expected_attempt in 1..=3 {
            assert!(retry.can_attempt());
            retry.record_failure();
            assert_eq!(retry.attempts, expected_attempt);
            if expected_attempt < 3 {
                retry.next_attempt = Some(Instant::now() - Duration::from_millis(1));
            }
        }
        assert!(!retry.can_attempt());
        retry.reset_if_key_changed("ice:two".to_string());
        assert!(retry.can_attempt());
        assert_eq!(retry.attempts, 0);
    }

    #[test]
    fn deterministic_failure_pauses_without_losing_its_reason() {
        let mut retry = RetryControl::default();
        retry.reset_if_key_changed("sky:one".into());
        retry.record_error("CODEDROBE_DOM_INCOMPATIBLE");
        retry.next_attempt = Some(Instant::now() - Duration::from_secs(30));
        assert!(!retry.can_attempt());
        assert_eq!(retry.wait_status().1, Some("CODEDROBE_DOM_INCOMPATIBLE"));
        assert!(retry.wait_status().0.contains("暂停"));
        retry.reset_if_key_changed("sky:two".into());
        assert!(retry.can_attempt());
    }

    #[test]
    fn temporary_failure_waits_and_exhaustion_retains_original_code() {
        let mut retry = RetryControl::default();
        retry.record_error("CODEDROBE_CDP_TIMEOUT");
        assert!(retry.wait_status().0.contains("等待"));
        assert_eq!(retry.wait_status().1, Some("CODEDROBE_CDP_TIMEOUT"));
        retry.next_attempt = None;
        assert!(retry.can_attempt());
        retry.record_error("CODEDROBE_CDP_TIMEOUT");
        retry.record_error("CODEDROBE_CDP_TIMEOUT");
        assert!(!retry.can_attempt());
        assert!(retry.wait_status().0.contains("停止"));
        assert_eq!(retry.wait_status().1, Some("CODEDROBE_CDP_TIMEOUT"));
    }

    #[test]
    fn a_new_renderer_observation_is_debounced_before_auto_apply() {
        let mut retry = RetryControl::default();
        assert!(retry.reset_if_key_changed("sky:renderer-a".to_string()));
        retry.defer_initial_attempt(Duration::from_secs(1));
        assert!(!retry.can_attempt());

        retry.next_attempt = Some(Instant::now() - Duration::from_millis(1));
        assert!(retry.can_attempt());

        retry.reset();
        assert!(retry.reset_if_key_changed("sky:renderer-a".to_string()));
    }

    #[test]
    fn log_values_cannot_span_multiple_lines() {
        assert_eq!(safe_log_value("apply\r\nsecret"), "apply  secret");
    }

    #[test]
    fn a_workbuddy_path_with_unicode_and_spaces_is_accepted() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("中文 WorkBuddy");
        fs::create_dir_all(root.join("resources")).unwrap();
        fs::write(root.join("WorkBuddy.exe"), []).unwrap();
        fs::write(root.join("resources").join("app.asar"), []).unwrap();
        assert!(validate_workbuddy_path(&root.join("WorkBuddy.exe")).is_ok());
    }

    #[test]
    fn automatic_drive_candidates_cover_custom_install_drives() {
        assert_eq!(
            drive_root_workbuddy_candidates(['D', 'E']),
            vec![
                PathBuf::from(r"D:\WorkBuddy\WorkBuddy.exe"),
                PathBuf::from(r"D:\Program Files\WorkBuddy\WorkBuddy.exe"),
                PathBuf::from(r"E:\WorkBuddy\WorkBuddy.exe"),
                PathBuf::from(r"E:\Program Files\WorkBuddy\WorkBuddy.exe"),
            ]
        );
    }

    #[test]
    fn supports_current_and_newer_workbuddy_5x_versions() {
        assert!(!is_supported_workbuddy_version(Some("5.2.5.0")));
        assert!(is_supported_workbuddy_version(Some("5.2.6.0")));
        assert!(is_supported_workbuddy_version(Some("5.2.8.0")));
        assert!(is_supported_workbuddy_version(Some("5.3.14.0")));
        assert!(!is_supported_workbuddy_version(Some("6.0.0.0")));
        assert!(is_supported_workbuddy_version(Some("unknown")));
    }

    #[test]
    fn catalog_fails_with_a_clear_error_when_a_package_is_missing() {
        let temporary = tempfile::tempdir().unwrap();
        let body = r#"[{"id":"ice","name":"Ice","description":"x","packagePath":"themes/ice/theme.codedrobe-theme","previewPath":"themes/ice/preview.png","verifiedWorkBuddyVersion":"5.2.6","themeVersion":"1","runtimeThemeId":"workbuddy-ice"}]"#;
        fs::write(temporary.path().join("themes.json"), body).unwrap();
        assert_eq!(
            load_catalog(temporary.path()).unwrap_err().code,
            "THEME_PACKAGE_MISSING"
        );
    }

    #[test]
    fn custom_theme_controls_and_hex_values_are_bounded() {
        assert!(validate_controls(&ThemeControls::default()).is_ok());
        assert!(validate_controls(&ThemeControls {
            brightness: 41,
            ..ThemeControls::default()
        })
        .is_err());
        assert!(validate_controls(&ThemeControls {
            blur: 25,
            ..ThemeControls::default()
        })
        .is_err());
        assert!(validate_controls(&ThemeControls {
            panel_opacity: 54,
            ..ThemeControls::default()
        })
        .is_err());
        assert!(validate_controls(&ThemeControls {
            accent: Some("#ff33aa".to_string()),
            ..ThemeControls::default()
        })
        .is_ok());
        assert!(validate_controls(&ThemeControls {
            accent: Some("not-a-color".to_string()),
            ..ThemeControls::default()
        })
        .is_err());
    }

    #[test]
    fn custom_image_signatures_must_match_supported_extensions() {
        assert_eq!(
            source_image_extension(
                Path::new("wallpaper.png"),
                &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
            )
            .unwrap(),
            "png"
        );
        assert!(source_image_extension(
            Path::new("wallpaper.jpg"),
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
        )
        .is_err());
        assert_eq!(
            source_image_extension(Path::new("wallpaper.webp"), b"RIFFxxxxWEBPpayload").unwrap(),
            "webp"
        );
    }

    #[test]
    fn base64_encoder_matches_standard_padding() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
    }

    #[test]
    fn base64_decoder_accepts_only_canonical_image_payloads() {
        assert_eq!(base64_decode("Zg=="), Some(b"f".to_vec()));
        assert_eq!(base64_decode("Zm8="), Some(b"fo".to_vec()));
        assert_eq!(base64_decode("Zm9v"), Some(b"foo".to_vec()));
        assert_eq!(base64_decode("Zg=A"), None);
        assert_eq!(base64_decode("Zh=="), None);
        assert_eq!(base64_decode("Zg==Zm8="), None);
        assert_eq!(base64_decode("not-base64"), None);
    }

    #[test]
    fn custom_theme_identifiers_are_confined_to_their_own_directory() {
        assert!(is_safe_custom_theme_id("custom-123-ff"));
        assert!(!is_safe_custom_theme_id("ice-blue"));
        assert!(!is_safe_custom_theme_id("custom-../escape"));
    }

    #[test]
    fn unavailable_cdp_probe_returns_before_the_ui_refresh_interval() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let _keep_connection_open = stream;
                std::thread::sleep(Duration::from_secs(3));
            }
        });
        let started = Instant::now();
        assert!(discover_renderer(port).is_err());
        assert!(started.elapsed() < Duration::from_millis(750));
    }
}
