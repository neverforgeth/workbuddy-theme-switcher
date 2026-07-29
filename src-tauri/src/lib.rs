use std::{
    env,
    ffi::c_void,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    net::TcpStream,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
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
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tungstenite::{connect, stream::MaybeTlsStream, Message, WebSocket};

const DEFAULT_CDP_PORT: u16 = 9336;
const CODEDROBE_STYLE_ID: &str = "codedrobe-theme-style-workbuddy";
const RENDERER_PATH: &str = "resources/app.asar/renderer/index.html";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MONITOR_INTERVAL: Duration = Duration::from_millis(2_500);
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

#[derive(Debug, Clone, Deserialize)]
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
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ThemeDto {
    id: String,
    name: String,
    description: String,
    preview_path: String,
    verified_work_buddy_version: String,
    theme_version: String,
}

#[derive(Debug, Clone)]
struct ResolvedTheme {
    record: ThemeRecord,
    package_path: PathBuf,
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
}

impl Default for PersistentState {
    fn default() -> Self {
        Self {
            schema_version: 1,
            desired_state: DesiredState::Original,
            selected_theme_id: None,
            selected_theme_version: None,
            auto_keep_theme: false,
            workbuddy_path: None,
            preferred_port: DEFAULT_CDP_PORT,
            last_successful_apply_at: None,
            last_successful_verify_theme: None,
            last_auto_recovery_at: None,
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
}

impl RetryControl {
    fn reset_if_key_changed(&mut self, key: String) -> bool {
        if self.key.as_deref() != Some(&key) {
            self.key = Some(key);
            self.attempts = 0;
            self.next_attempt = None;
            true
        } else {
            false
        }
    }

    fn defer_initial_attempt(&mut self, delay: Duration) {
        self.next_attempt = Some(Instant::now() + delay);
    }

    fn can_attempt(&self) -> bool {
        self.attempts < RETRY_DELAYS_SECS.len() as u8
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
    }
}

pub fn run() {
    let background_mode = launched_in_background_mode();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .manage(AppState::default())
        .setup(move |app| {
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_theme_catalog,
            detect_workbuddy,
            get_runtime_status,
            set_workbuddy_path,
            set_auto_keep_theme,
            apply_theme,
            restore_theme,
            open_logs_directory,
        ])
        .run(tauri::generate_context!())
        .expect("error while running WorkBuddy Theme Switcher");
}

fn launched_in_background_mode() -> bool {
    env::args().any(|argument| argument.eq_ignore_ascii_case("--background"))
}

#[tauri::command]
fn get_theme_catalog(app: AppHandle) -> AppResult<Vec<ThemeDto>> {
    let root = resource_root(&app)?;
    load_catalog(&root).map(|themes| {
        themes
            .into_iter()
            .map(|theme| ThemeDto {
                id: theme.id,
                name: theme.name,
                description: theme.description,
                preview_path: root.join(theme.preview_path).to_string_lossy().to_string(),
                verified_work_buddy_version: theme.verified_work_buddy_version,
                theme_version: theme.theme_version,
            })
            .collect()
    })
}

#[tauri::command]
fn detect_workbuddy(app: AppHandle) -> AppResult<WorkBuddyStatus> {
    detect_workbuddy_inner(&app)
}

#[tauri::command]
fn get_runtime_status(app: AppHandle, state: State<'_, AppState>) -> AppResult<RuntimeStatus> {
    runtime_status(&app, &state)
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
fn apply_theme(
    app: AppHandle,
    state: State<'_, AppState>,
    theme_id: String,
    allow_restart: bool,
) -> AppResult<OperationResult> {
    with_manual_operation(&state, || {
        let result = apply_theme_locked(&app, &theme_id, allow_restart, ApplyOrigin::Manual)?;
        persist_manual_theme_choice(&app, &theme_id)?;
        update_monitor(&state.monitor, "主题已应用并验证", 0, None);
        Ok(result)
    })
}

#[tauri::command]
fn restore_theme(app: AppHandle, state: State<'_, AppState>) -> AppResult<OperationResult> {
    with_manual_operation(&state, || {
        // Persist the user's explicit choice before any CDP work. The monitor therefore can never
        // race a restore and inject the theme again if the restore itself encounters an error.
        persist_original_choice(&app)?;
        set_login_autostart(&app, false)?;
        update_monitor(&state.monitor, "已恢复原版，自动保持已停止", 0, None);
        restore_theme_locked(&app)
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
    let catalog = load_catalog(&resource_root(app)?).unwrap_or_default();
    let current_theme_id = metadata.as_ref().and_then(|item| {
        item.runtime_theme_id.as_ref().map(|runtime_id| {
            catalog
                .iter()
                .find(|theme| theme.runtime_theme_id == *runtime_id)
                .map(|theme| theme.id.clone())
                .unwrap_or_else(|| runtime_id.clone())
        })
    });
    let message = match (running, cdp_available, current_theme_id.as_deref()) {
        (false, _, _) => "WorkBuddy 未运行。开启自动保持时会以本地 CDP 模式静默启动。".to_string(),
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
    let root = resource_root(app)?;
    let theme = resolve_theme(&root, theme_id)?;
    let status = detect_workbuddy_inner(app)?;
    let workbuddy_path = status
        .path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| AppError::new("WORKBUDDY_NOT_FOUND", "未找到有效的 WorkBuddy.exe。"))?;
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
        while !monitor.stop_requested.load(Ordering::SeqCst) {
            monitor_tick(
                &app,
                &operation_lock,
                &manual_operation_pending,
                &monitor,
                background_mode,
                &mut retry,
            );
            std::thread::sleep(MONITOR_INTERVAL);
        }
    });
}

fn monitor_tick(
    app: &AppHandle,
    operation_lock: &Arc<Mutex<()>>,
    manual_operation_pending: &Arc<AtomicBool>,
    monitor: &Arc<MonitorRuntime>,
    background_mode: bool,
    retry: &mut RetryControl,
) {
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
            update_monitor(
                monitor,
                "自动应用失败",
                retry.attempts,
                Some("RETRY_LIMIT_REACHED"),
            );
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
                retry.record_failure();
                update_monitor(monitor, "自动应用失败", retry.attempts, Some(&error.code));
            }
        }
        return;
    }

    let target = match discover_renderer(port) {
        Ok(target) => target,
        Err(_) if background_mode => {
            // Login recovery may restart an unconfigured WorkBuddy once. In an interactive session
            // we never close the user's already-running WorkBuddy without their confirmation.
            retry.reset_if_key_changed(format!("{}:cdp-unavailable", theme_id));
            if !retry.can_attempt() || manual_operation_pending.load(Ordering::SeqCst) {
                return;
            }
            let Ok(lock) = operation_lock.try_lock() else {
                return;
            };
            if !auto_keep_matches(app, &theme_id) {
                drop(lock);
                return;
            }
            update_monitor(monitor, "等待 CDP，正在安全重启", retry.attempts, None);
            let restarted = request_graceful_workbuddy_close()
                .and_then(|_| launch_workbuddy(&workbuddy_path, port))
                .and_then(|_| wait_for_renderer(port));
            drop(lock);
            match restarted {
                Ok(()) => {
                    retry.reset();
                    update_monitor(monitor, "已连接，正在检查主题", 0, None);
                }
                Err(error) => {
                    retry.record_failure();
                    update_monitor(monitor, "自动应用失败", retry.attempts, Some(&error.code));
                }
            }
            return;
        }
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
    let expected = match resolve_theme(&resource_root(app).unwrap_or_default(), &theme_id) {
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
        update_monitor(
            monitor,
            "自动应用失败",
            retry.attempts,
            Some("RETRY_LIMIT_REACHED"),
        );
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
            let workbuddy_version = read_workbuddy_version(&workbuddy_path);
            let _ = write_log(
                app,
                "auto-reapply",
                "failed",
                Some(&theme_id),
                workbuddy_path.to_str(),
                workbuddy_version.as_deref(),
                &error.code,
            );
            retry.record_failure();
            update_monitor(monitor, "自动应用失败", retry.attempts, Some(&error.code));
        }
    }
}

fn auto_keep_matches(app: &AppHandle, theme_id: &str) -> bool {
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
    if saved.preferred_port == 0 {
        saved.preferred_port = DEFAULT_CDP_PORT;
    }
    if saved.selected_theme_id.is_none() {
        saved.auto_keep_theme = false;
    }
    saved.schema_version = 1;
    let _ = app;
    Ok(saved)
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
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
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

fn resolve_theme(root: &Path, requested_id: &str) -> AppResult<ResolvedTheme> {
    let record = load_catalog(root)?
        .into_iter()
        .find(|theme| theme.id == requested_id)
        .ok_or_else(|| AppError::new("THEME_NOT_FOUND", "所选主题不存在。"))?;
    let package_path = resolve_resource_path(root, &record.package_path)?;
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
    let saved = load_persistent_state(app)?.workbuddy_path;
    let mut candidates = Vec::new();
    if let Some(path) = saved {
        candidates.push(path);
    }
    candidates.push(PathBuf::from(r"D:\workbuddy\WorkBuddy.exe"));
    if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(&local_app_data)
                .join("Programs")
                .join("WorkBuddy")
                .join("WorkBuddy.exe"),
        );
        candidates.push(
            PathBuf::from(local_app_data)
                .join("WorkBuddy")
                .join("WorkBuddy.exe"),
        );
    }
    if let Some(program_files) = env::var_os("PROGRAMFILES") {
        candidates.push(
            PathBuf::from(program_files)
                .join("WorkBuddy")
                .join("WorkBuddy.exe"),
        );
    }
    Ok(candidates
        .into_iter()
        .find(|path| validate_workbuddy_path(path).is_ok()))
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

fn configure_hidden_command(command: &mut Command) {
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let _ = command;
}

fn hidden_output(program: &str, args: &[&str]) -> io::Result<std::process::Output> {
    let mut command = Command::new(program);
    configure_hidden_command(&mut command);
    command.args(args).stdin(Stdio::null()).output()
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
        .try_proxy_from_env(false)
        .timeout(Duration::from_secs(2))
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
    let (mut socket, _) = connect(target.websocket_debugger_url.as_str())
        .map_err(|_| AppError::new("CDP_UNAVAILABLE", "无法连接 WorkBuddy renderer。"))?;
    set_socket_timeout(&mut socket)?;
    // This only returns node count and CodeDrobe's own theme identifier. It never reads DOM text,
    // user input, account details, or message content.
    let expression = format!("(() => ({{ styleNodeCount: document.querySelectorAll('#{}').length, runtimeThemeId: document.documentElement?.dataset?.codedrobeTheme ?? null }}))()", CODEDROBE_STYLE_ID);
    socket.send(Message::Text(json!({ "id": 1, "method": "Runtime.evaluate", "params": { "expression": expression, "returnByValue": true } }).to_string().into()))
        .map_err(|_| AppError::new("CDP_UNAVAILABLE", "无法向 WorkBuddy renderer 发送检查请求。"))?;
    loop {
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
                .set_read_timeout(Some(Duration::from_secs(5)))
                .map_err(|_| AppError::new("CDP_UNAVAILABLE", "无法设置 WorkBuddy CDP 超时。"))?;
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
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
    let output = command
        .arg(&cli)
        .args(args)
        .current_dir(
            cli.parent()
                .and_then(Path::parent)
                .unwrap_or_else(|| Path::new(".")),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                AppError::new("NODE_NOT_FOUND", "内置 Node.js 运行时不可用。")
            } else {
                AppError::new("CODEDROBE_COMMAND_FAILED", "无法启动本地 CodeDrobe Core。")
            }
        })?;
    if !output.status.success() {
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let code = known_codedrobe_error_code(&combined).unwrap_or("CODEDROBE_COMMAND_FAILED");
        let process_code = output
            .status
            .code()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "terminated".to_string());
        let diagnostic = format!("{}-EXIT{}", code, process_code);
        // Keep only a static error code and exit status; command output can include target details
        // and is intentionally never written to disk.
        let _ = write_log(app, "codedrobe", "process-failed", None, None, None, &diagnostic);
        return Err(AppError::new(code, codedrobe_error_message(code)));
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
        .ok_or_else(|| AppError::new(
            "CODEDROBE_NOT_FOUND",
            "内置 CodeDrobe Core 不完整。请重新安装主题切换器。",
        ))
}

fn find_node_runtime(app: &AppHandle) -> AppResult<PathBuf> {
    let root = resource_root(app)?;
    bundled_runtime_candidates(&root, "node/node.exe")
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| AppError::new(
            "NODE_NOT_FOUND",
            "内置 Node.js 运行时不完整。请重新安装主题切换器。",
        ))
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

fn known_codedrobe_error_code(output: &str) -> Option<&'static str> {
    [
        "CODEDROBE_RESTART_REQUIRED",
        "CODEDROBE_VERIFY_FAILED",
        "CODEDROBE_DOM_INCOMPATIBLE",
        "CODEDROBE_PORT_OCCUPIED",
        "TARGET_NOT_FOUND",
        "NOT_CONNECTED",
    ]
    .into_iter()
    .find(|code| output.contains(code))
}

fn codedrobe_error_message(code: &str) -> &'static str {
    match code {
        "CODEDROBE_RESTART_REQUIRED" => "WorkBuddy 需要重启后才能开启本地 CDP。",
        "CODEDROBE_VERIFY_FAILED" => "主题已尝试应用，但 CodeDrobe 验证未通过。",
        "CODEDROBE_DOM_INCOMPATIBLE" => "WorkBuddy 页面仍在加载，主题暂时无法应用。",
        "CODEDROBE_PORT_OCCUPIED" => "本地 CDP 端口已被其他程序占用。",
        "TARGET_NOT_FOUND" | "NOT_CONNECTED" => "未找到可用的 WorkBuddy renderer。",
        _ => "本地 CodeDrobe Core 未能完成操作，请查看日志目录。",
    }
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

fn persist_manual_theme_choice(app: &AppHandle, theme_id: &str) -> AppResult<()> {
    let theme = resolve_theme(&resource_root(app)?, theme_id)?;
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
    fn catalog_fails_with_a_clear_error_when_a_package_is_missing() {
        let temporary = tempfile::tempdir().unwrap();
        let body = r#"[{"id":"ice","name":"Ice","description":"x","packagePath":"themes/ice/theme.codedrobe-theme","previewPath":"themes/ice/preview.png","verifiedWorkBuddyVersion":"5.2.6","themeVersion":"1","runtimeThemeId":"workbuddy-ice"}]"#;
        fs::write(temporary.path().join("themes.json"), body).unwrap();
        assert_eq!(
            load_catalog(temporary.path()).unwrap_err().code,
            "THEME_PACKAGE_MISSING"
        );
    }
}
