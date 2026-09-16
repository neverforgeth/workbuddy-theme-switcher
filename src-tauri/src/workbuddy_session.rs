//! Async command seam and durable trial coordinator. Only host mutations share operation_lock.
use super::*;
use std::collections::HashMap;
use tauri::Emitter;
use theme_library::{DraftUpdate, DraftView, LibraryItem, ThemeDocument, ThemeRef, ThemeStore};

#[derive(Default)]
pub(crate) struct StudioState {
    pub store_lock: Mutex<()>,
    snapshot: Mutex<Option<RuntimeSnapshot>>,
    trial: Mutex<Option<TrialJournal>>,
    pub trial_pending: AtomicBool,
    jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub operation: Mutex<Option<String>>,
    last_sample: Mutex<Option<Instant>>,
    sampled_at: Mutex<Option<Instant>>,
    last_recovery: Mutex<Option<Instant>>,
    recovery_error: Mutex<Option<String>>,
    close_requested: AtomicBool,
    window_epoch: Mutex<u64>,
    capture: Mutex<Option<live_preview::CapturedFrame>>,
    // Read-only screenshot work must never hold the host apply/recovery mutex.
    capture_gate: Mutex<()>,
    capture_at: Mutex<Option<Instant>>,
}
impl StudioState {
    fn start_epoch(&self) -> AppResult<u64> {
        self.window_epoch
            .lock()
            .map(|epoch| *epoch)
            .map_err(|_| lock_error())
    }
    fn begin_start(&self, requested_epoch: u64) -> AppResult<()> {
        let epoch = self.window_epoch.lock().map_err(|_| lock_error())?;
        if *epoch != requested_epoch {
            return Err(AppError::new(
                "TRIAL_WINDOW_CLOSED",
                "窗口已关闭，已取消排队的试穿请求。",
            ));
        }
        self.close_requested.store(false, Ordering::SeqCst);
        Ok(())
    }
    fn mark_closed(&self) {
        // Serialize close against reopening for a newly dispatched (not queued old) start.
        if let Ok(mut epoch) = self.window_epoch.lock() {
            *epoch = epoch.wrapping_add(1);
            self.close_requested.store(true, Ordering::SeqCst);
        } else {
            self.close_requested.store(true, Ordering::SeqCst);
        }
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeSnapshot {
    pub workbuddy: WorkBuddyStatus,
    pub runtime: RuntimeStatus,
    pub trial: Option<TrialSession>,
    pub operation: Option<String>,
    pub captured_at: String,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrialSession {
    pub id: String,
    pub name: String,
    pub phase: String,
    pub deadline_ms: i64,
    pub error: Option<String>,
    #[serde(default)]
    pub synced_sequence: Option<u64>,
    #[serde(default)]
    pub css_hash: Option<String>,
}
impl TrialSession {
    fn live(id: String, name: String, now: i64, sequence: u64) -> Self {
        Self {
            id,
            name,
            phase: "active".into(),
            deadline_ms: now + 600_000,
            error: None,
            synced_sequence: Some(sequence),
            css_hash: None,
        }
    }
    fn renew(&mut self, now: i64) -> AppResult<()> {
        if self.phase != "active" || now >= self.deadline_ms {
            return Err(AppError::new("TRIAL_EXPIRED", "试穿已结束，不能续期。"));
        }
        self.deadline_ms = self
            .deadline_ms
            .checked_add(600_000)
            .ok_or_else(lock_error)?;
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrialJournal {
    session: TrialSession,
    candidate: ThemeRecord,
    draft: Option<ThemeDocument>,
    previous_key: Option<String>,
    previous_intent: PersistentState,
    committed: bool,
    #[serde(default)]
    commit_reference: Option<ThemeRef>,
    #[serde(default)]
    target_id: Option<String>,
}
fn lock_error() -> AppError {
    AppError::new("STUDIO_BUSY", "工作台状态暂不可用，请重试。")
}
fn journal_path() -> AppResult<PathBuf> {
    Ok(ThemeStore::production()?.root.join("trial.json"))
}
fn persist_trial(journal: &TrialJournal) -> AppResult<()> {
    let path = journal_path()?;
    fs::create_dir_all(path.parent().unwrap()).map_err(|_| lock_error())?;
    atomic_write(
        &path,
        &serde_json::to_vec(journal).map_err(|_| lock_error())?,
    )
    .map_err(|_| AppError::new("TRIAL_WRITE_FAILED", "无法记录试用恢复信息，未开始试用。"))
}
fn close_guard(session: &mut TrialSession, closed: bool) -> bool {
    if closed && session.phase == "active" {
        session.phase = "pendingRecovery".into();
        session.error = Some("TRIAL_WINDOW_CLOSED".into());
        true
    } else {
        false
    }
}
fn set_trial(app: &AppHandle, mut journal: Option<TrialJournal>) -> AppResult<()> {
    let studio = app.state::<StudioState>();
    let mut current = studio.trial.lock().map_err(|_| lock_error())?;
    // Closing cannot be undone by a start/renew/sync that cloned the journal earlier.
    let closed = journal.as_mut().is_some_and(|j| {
        close_guard(
            &mut j.session,
            studio.close_requested.load(Ordering::SeqCst),
        )
    });
    if let Some(ref record) = journal {
        persist_trial(record)?;
    } else {
        let path = journal_path()?;
        if path.exists() {
            fs::remove_file(path)
                .map_err(|_| AppError::new("TRIAL_WRITE_FAILED", "无法清除试用恢复记录。"))?;
        }
    }
    studio
        .trial_pending
        .store(journal.is_some(), Ordering::SeqCst);
    *current = journal;
    drop(current);
    if !studio.trial_pending.load(Ordering::SeqCst) {
        if let Ok(mut capture) = studio.capture.lock() {
            *capture = None;
        }
        if let Ok(mut at) = studio.capture_at.lock() {
            *at = None;
        }
    }
    publish_state(app);
    if closed {
        return Err(AppError::new(
            "TRIAL_WINDOW_CLOSED",
            "编辑窗口已关闭，正在恢复之前的效果。",
        ));
    }
    Ok(())
}
pub(crate) fn has_trial(app: &AppHandle) -> bool {
    app.try_state::<StudioState>()
        .is_some_and(|s| s.trial_pending.load(Ordering::SeqCst))
}
fn publish_state(app: &AppHandle) {
    let studio = app.state::<StudioState>();
    let trial = studio
        .trial
        .lock()
        .ok()
        .and_then(|j| j.as_ref().map(|j| j.session.clone()));
    let operation = studio.operation.lock().ok().and_then(|j| j.clone());
    if let Ok(mut cached) = studio.snapshot.lock() {
        if let Some(ref mut snapshot) = *cached {
            snapshot.trial = trial;
            snapshot.operation = operation;
            snapshot.captured_at = now_timestamp();
            let _ = app.emit("studio-runtime", snapshot.clone());
        }
    };
}
fn host_operation<T>(
    app: &AppHandle,
    managed: &AppState,
    operation: impl FnOnce() -> AppResult<T>,
) -> AppResult<T> {
    with_manual_operation(managed, || {
        let studio = app.state::<StudioState>();
        *studio.operation.lock().map_err(|_| lock_error())? = Some("正在同步 WorkBuddy".into());
        publish_state(app);
        let result = operation();
        *studio.operation.lock().map_err(|_| lock_error())? = None;
        publish_state(app);
        result
    })
}
fn require_no_trial(app: &AppHandle) -> AppResult<()> {
    if has_trial(app) {
        Err(AppError::new(
            "TRIAL_ACTIVE",
            "请先保留或恢复实机试用；待恢复时不能开始新的主题操作。",
        ))
    } else {
        Ok(())
    }
}
pub(crate) fn resolve_trial(app: &AppHandle, key: &str) -> AppResult<ResolvedTheme> {
    let studio = app.state::<StudioState>();
    let journal = studio.trial.lock().map_err(|_| lock_error())?;
    let j = journal
        .as_ref()
        .filter(|j| j.candidate.id == key)
        .ok_or_else(|| AppError::new("THEME_NOT_FOUND", "试用主题已失效。"))?;
    Ok(ResolvedTheme {
        record: j.candidate.clone(),
        package_path: PathBuf::from(&j.candidate.package_path),
    })
}
pub(crate) async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|_| AppError::new("TASK_FAILED", "后台任务异常结束。"))?
}

#[tauri::command]
pub(crate) async fn studio_library(app: AppHandle) -> AppResult<Vec<LibraryItem>> {
    blocking(move || {
        let mut result: Vec<LibraryItem> = theme_catalog(&app)?
            .into_iter()
            .filter(|t| !t.id.starts_with("custom-v2-"))
            .map(|t| LibraryItem {
                reference: ThemeRef {
                    id: t.id,
                    revision: None,
                },
                name: t.name,
                description: t.description,
                preview_path: t.preview_path,
                editable: false,
                is_custom: t.is_custom,
                palette: None,
                compatibility: "内置/旧主题；应用时运行验证".into(),
            })
            .collect();
        result.extend(ThemeStore::production()?.list()?);
        Ok(result)
    })
    .await
}
#[tauri::command]
pub(crate) async fn studio_import(
    app: AppHandle,
    filename: String,
    data: String,
    job_id: Option<String>,
) -> AppResult<DraftView> {
    let job_id = job_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    run_job(app.clone(), job_id.clone(), move |token| {
        let started = Instant::now();
        let _ = app.emit("studio-job", json!({"id":job_id,"phase":"decoding"}));
        let bytes = decode_image_base64(
            &data,
            theme_engine::MAX_IMAGE_BYTES,
            "IMAGE_SIZE_INVALID",
            "图片不得超过 10MB。",
        )?;
        job_checkpoint(&token, started.elapsed(), Duration::from_secs(20))?;
        let image = theme_engine::import_image(&filename, &bytes)?;
        job_checkpoint(&token, started.elapsed(), Duration::from_secs(20))?;
        let _ = app.emit("studio-job", json!({"id":job_id,"phase":"persisting"}));
        let studio = app.state::<StudioState>();
        let _lock = studio.store_lock.lock().map_err(|_| lock_error())?;
        job_checkpoint(&token, started.elapsed(), Duration::from_secs(20))?;
        ThemeStore::production()?.import_processed(&filename, &bytes, image)
    })
    .await
}
fn job_checkpoint(cancelled: &AtomicBool, elapsed: Duration, limit: Duration) -> AppResult<()> {
    if cancelled.load(Ordering::SeqCst) {
        return Err(AppError::new(
            "TASK_CANCELLED",
            "任务已取消，未提交新结果。",
        ));
    }
    if elapsed >= limit {
        return Err(AppError::new(
            "TASK_TIMEOUT",
            "处理超过时间限制，未提交新结果。",
        ));
    }
    Ok(())
}
async fn run_job<T: Send + 'static>(
    app: AppHandle,
    job_id: String,
    operation: impl FnOnce(Arc<AtomicBool>) -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    if uuid::Uuid::parse_str(&job_id).is_err() {
        return Err(AppError::new("JOB_ID_INVALID", "任务标识无效。"));
    }
    let token = Arc::new(AtomicBool::new(false));
    {
        let studio = app.state::<StudioState>();
        let mut jobs = studio.jobs.lock().map_err(|_| lock_error())?;
        if !jobs.is_empty() {
            return Err(AppError::new("TASK_BUSY", "已有图片任务正在进行或取消中。"));
        }
        jobs.insert(job_id.clone(), token.clone());
    }
    let result = blocking(move || operation(token)).await;
    if let Ok(mut jobs) = app.state::<StudioState>().jobs.lock() {
        jobs.remove(&job_id);
    }
    let _ = app.emit("studio-job", json!({"id":job_id,"phase":"finished"}));
    result
}
#[tauri::command]
pub(crate) async fn studio_latest_draft() -> AppResult<Option<DraftView>> {
    blocking(|| ThemeStore::production()?.latest()).await
}
#[tauri::command]
pub(crate) async fn studio_update_draft(
    app: AppHandle,
    draft_id: String,
    update: DraftUpdate,
) -> AppResult<DraftView> {
    blocking(move || {
        let studio = app.state::<StudioState>();
        let _lock = studio.store_lock.lock().map_err(|_| lock_error())?;
        ThemeStore::production()?.update(&draft_id, update)
    })
    .await
}
#[tauri::command]
pub(crate) async fn studio_update_design(
    app: AppHandle,
    draft_id: String,
    sequence: u64,
    design: Option<region_theme::DesignAdvice>,
) -> AppResult<DraftView> {
    blocking(move || {
        let studio = app.state::<StudioState>();
        let _lock = studio.store_lock.lock().map_err(|_| lock_error())?;
        ThemeStore::production()?.update_design(&draft_id, sequence, design)
    })
    .await
}
#[tauri::command]
pub(crate) async fn studio_save_draft(app: AppHandle, draft_id: String) -> AppResult<ThemeRef> {
    blocking(move || {
        let studio = app.state::<StudioState>();
        let _lock = studio.store_lock.lock().map_err(|_| lock_error())?;
        ThemeStore::production()?.save(&draft_id)
    })
    .await
}
#[tauri::command]
pub(crate) async fn studio_open_theme(app: AppHandle, reference: ThemeRef) -> AppResult<DraftView> {
    blocking(move || {
        let studio = app.state::<StudioState>();
        let _lock = studio.store_lock.lock().map_err(|_| lock_error())?;
        ThemeStore::production()?.open(&reference)
    })
    .await
}
#[tauri::command]
pub(crate) async fn studio_copy_legacy(
    app: AppHandle,
    reference: ThemeRef,
) -> AppResult<DraftView> {
    blocking(move || {
        let resolved = resolve_theme(&app, &reference.key())?;
        if !resolved.record.is_custom {
            return Err(AppError::new("BUILTIN_READONLY", "内置主题保持只读。"));
        }
        let path = PathBuf::from(resolved.record.preview_path);
        let bytes = fs::read(&path)
            .map_err(|_| AppError::new("IMAGE_READ_FAILED", "无法读取旧主题图片。"))?;
        let studio = app.state::<StudioState>();
        let _lock = studio.store_lock.lock().map_err(|_| lock_error())?;
        let mut view = ThemeStore::production()?.import(
            &path.file_name().unwrap_or_default().to_string_lossy(),
            &bytes,
        )?;
        view.warning = Some("已从旧主题图片建立新草稿；旧主题及其效果保持不变。".into());
        Ok(view)
    })
    .await
}
#[tauri::command]
pub(crate) async fn studio_copy_fusion(
    app: AppHandle,
    draft_id: String,
    job_id: String,
) -> AppResult<DraftView> {
    run_job(app.clone(), job_id, move |token| {
        require_no_trial(&app)?;
        let started = Instant::now();
        let studio = app.state::<StudioState>();
        let _lock = studio.store_lock.lock().map_err(|_| lock_error())?;
        ThemeStore::production()?.copy_fusion(&draft_id, || {
            require_no_trial(&app)?;
            job_checkpoint(&token, started.elapsed(), Duration::from_secs(20))
        })
    })
    .await
}
#[tauri::command]
pub(crate) async fn studio_rename_theme(
    app: AppHandle,
    reference: ThemeRef,
    name: String,
) -> AppResult<ThemeRef> {
    blocking(move || {
        require_no_trial(&app)?;
        let studio = app.state::<StudioState>();
        let _lock = studio.store_lock.lock().map_err(|_| lock_error())?;
        ThemeStore::production()?.rename(&reference, &name)
    })
    .await
}
#[tauri::command]
pub(crate) async fn studio_delete_theme(app: AppHandle, reference: ThemeRef) -> AppResult<()> {
    blocking(move || {
        require_no_trial(&app)?;
        let managed = app.state::<AppState>();
        with_manual_operation(&managed, || {
            require_no_trial(&app)?;
            let saved = load_persistent_state(&app)?;
            let status = detect_workbuddy_inner(&app)?;
            let same = |key: &str| ThemeRef::parse(key).is_ok_and(|v| v.id == reference.id);
            if saved.selected_theme_id.as_deref().is_some_and(same)
                || status.current_theme_id.as_deref().is_some_and(same)
            {
                return Err(AppError::new(
                    "THEME_ACTIVE",
                    "请先应用其他主题或恢复原版，再删除此主题。",
                ));
            }
            let studio = app.state::<StudioState>();
            let _lock = studio.store_lock.lock().map_err(|_| lock_error())?;
            if reference.revision.is_some() {
                ThemeStore::production()?.trash(&reference.id)
            } else {
                let resolved = resolve_theme(&app, &reference.key())?;
                if !resolved.record.is_custom {
                    return Err(AppError::new("BUILTIN_READONLY", "内置主题不能删除。"));
                }
                let source = custom_theme_directory(&reference.id)?
                    .canonicalize()
                    .map_err(|_| lock_error())?;
                if source.parent()
                    != Some(
                        custom_theme_root()?
                            .canonicalize()
                            .map_err(|_| lock_error())?
                            .as_path(),
                    )
                {
                    return Err(lock_error());
                }
                let trash = ThemeStore::production()?.root.join("trash");
                fs::create_dir_all(&trash).map_err(|_| lock_error())?;
                fs::rename(
                    source,
                    trash.join(format!(
                        "{}-{}",
                        reference.id,
                        uuid::Uuid::new_v4().simple()
                    )),
                )
                .map_err(|_| lock_error())
            }
        })
    })
    .await
}

#[tauri::command]
pub(crate) fn studio_cancel_job(app: AppHandle, job_id: String) -> AppResult<()> {
    if let Some(token) = app
        .state::<StudioState>()
        .jobs
        .lock()
        .map_err(|_| lock_error())?
        .get(&job_id)
    {
        token.store(true, Ordering::SeqCst);
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn studio_apply(
    app: AppHandle,
    reference: ThemeRef,
    allow_restart: bool,
) -> AppResult<OperationResult> {
    blocking(move || {
        require_no_trial(&app)?;
        let managed = app.state::<AppState>();
        host_operation(&app, &managed, || {
            require_no_trial(&app)?;
            let result =
                apply_theme_locked(&app, &reference.key(), allow_restart, ApplyOrigin::Manual)?;
            persist_manual_theme_choice(&app, &reference.key())?;
            Ok(result)
        })
    })
    .await
}
#[tauri::command]
pub(crate) async fn studio_restore(app: AppHandle) -> AppResult<OperationResult> {
    blocking(move || {
        require_no_trial(&app)?;
        let managed = app.state::<AppState>();
        host_operation(&app, &managed, || {
            require_no_trial(&app)?;
            persist_original_choice(&app)?;
            set_login_autostart(&app, false)?;
            restore_theme_locked(&app)
        })
    })
    .await
}
#[tauri::command]
pub(crate) async fn studio_set_path(app: AppHandle, path: String) -> AppResult<WorkBuddyStatus> {
    blocking(move || {
        require_no_trial(&app)?;
        let managed = app.state::<AppState>();
        with_manual_operation(&managed, || {
            require_no_trial(&app)?;
            set_workbuddy_path(app.clone(), path)
        })
    })
    .await
}
#[tauri::command]
pub(crate) async fn studio_auto_keep(app: AppHandle, enabled: bool) -> AppResult<RuntimeStatus> {
    blocking(move || {
        require_no_trial(&app)?;
        set_auto_keep_theme(app.clone(), app.state::<AppState>(), enabled)
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThemePreview {
    pub name: String,
    pub css: String,
    pub image_path: String,
    pub compiled: Option<theme_engine::CompiledTheme>,
}
#[tauri::command]
pub(crate) async fn studio_preview_theme(
    app: AppHandle,
    reference: ThemeRef,
) -> AppResult<ThemePreview> {
    blocking(move || {
        if reference.revision.is_some() {
            let store = ThemeStore::production()?;
            let doc = store.read_revision(&reference)?;
            return Ok(ThemePreview {
                name: doc.name,
                css: doc.compiled.css.clone(),
                image_path: store
                    .asset_file(&doc.asset_id, "hero.jpg")?
                    .to_string_lossy()
                    .into(),
                compiled: Some(doc.compiled),
            });
        }
        let resolved = resolve_theme(&app, &reference.key())?;
        let body: Value =
            serde_json::from_slice(&fs::read(resolved.package_path).map_err(|_| lock_error())?)
                .map_err(|_| lock_error())?;
        let css = body
            .pointer("/targets/workbuddy/css")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AppError::new("PREVIEW_UNAVAILABLE", "主题不包含可预览的 WorkBuddy 样式。")
            })?;
        Ok(ThemePreview {
            name: resolved.record.name,
            css: css.into(),
            image_path: resolved.record.preview_path,
            compiled: None,
        })
    })
    .await
}

#[tauri::command]
pub(crate) async fn studio_start_trial(
    app: AppHandle,
    draft_id: String,
    allow_restart: bool,
) -> AppResult<TrialSession> {
    let requested_epoch = app.state::<StudioState>().start_epoch()?;
    blocking(move || {
        require_no_trial(&app)?;
        let managed = app.state::<AppState>();
        host_operation(&app, &managed, || {
            require_no_trial(&app)?;
            let studio = app.state::<StudioState>();
            studio.begin_start(requested_epoch)?;
            let store = ThemeStore::production()?;
            let doc = {
                let _lock = studio.store_lock.lock().map_err(|_| lock_error())?;
                store.read_draft(&draft_id)?
            };
            let status = detect_workbuddy_inner(&app)?;
            if (!status.running || !status.cdp_available) && !allow_restart {
                return Err(AppError::new(
                    "RESTART_REQUIRED",
                    "实机试用需要先确认启动或重启 WorkBuddy。",
                ));
            }
            // Without CDP, do not assume we know the currently rendered theme. Connect first, then retry.
            if !status.running || !status.cdp_available {
                let path = resolve_workbuddy_path(&app)?
                    .ok_or_else(|| AppError::new("WORKBUDDY_NOT_FOUND", "未找到 WorkBuddy。"))?;
                if status.running {
                    request_graceful_workbuddy_close()?;
                }
                launch_workbuddy(&path, load_persistent_state(&app)?.preferred_port)?;
                wait_for_renderer(load_persistent_state(&app)?.preferred_port)?;
            }
            let before = detect_workbuddy_inner(&app)?;
            if !matches!(before.style_node_count, Some(0) | Some(1)) {
                return Err(AppError::new(
                    "TRIAL_BASELINE_UNKNOWN",
                    "无法确认原主题节点，未开始试用。",
                ));
            }
            let previous_key = if before.style_node_count == Some(1) {
                let key = before
                    .current_theme_id
                    .clone()
                    .ok_or_else(|| AppError::new("TRIAL_BASELINE_UNKNOWN", "当前主题无法识别。"))?;
                resolve_theme(&app, &key).map_err(|_| {
                    AppError::new(
                        "TRIAL_BASELINE_UNKNOWN",
                        "当前不是本工具可恢复的已知主题，未开始试用。",
                    )
                })?;
                Some(key)
            } else {
                None
            };
            let id = uuid::Uuid::new_v4().simple().to_string();
            let key = format!("trial-{id}");
            let directory = store.root.join("trials").join(&id);
            fs::create_dir_all(&directory).map_err(|_| lock_error())?;
            let package_path = directory.join("theme.codedrobe-theme");
            let runtime_id = format!("workbuddy-{key}");
            let hero =
                fs::read(store.asset_file(&doc.asset_id, "hero.jpg")?).map_err(|_| lock_error())?;
            atomic_write(
                &package_path,
                &serde_json::to_vec(&theme_engine::package(
                    &runtime_id,
                    &doc.name,
                    &doc.compiled,
                    &hero,
                ))
                .map_err(|_| lock_error())?,
            )
            .map_err(|_| lock_error())?;
            let candidate = ThemeRecord {
                id: key,
                name: doc.name.clone(),
                description: "临时试用".into(),
                package_path: package_path.to_string_lossy().into(),
                preview_path: store
                    .asset_file(&doc.asset_id, "hero.jpg")?
                    .to_string_lossy()
                    .into(),
                verified_work_buddy_version: "运行时验证".into(),
                theme_version: "1.4.0".into(),
                runtime_theme_id: runtime_id,
                is_custom: true,
            };
            // A known ID with externally modified CSS is not a recoverable known theme.
            let mut baseline_cdp =
                live_preview::Cdp::connect(load_persistent_state(&app)?.preferred_port)?;
            let (count, runtime, hash) = baseline_cdp.theme_fingerprint()?;
            let known = previous_key
                .as_deref()
                .map(|key| package_fingerprint(&app, key))
                .transpose()?
                .into_iter()
                .collect::<Vec<_>>();
            if !recovery_surface_is_known(
                count,
                runtime.as_deref(),
                hash.as_deref(),
                &known,
                previous_key.is_none(),
            ) {
                return Err(AppError::new(
                    "TRIAL_BASELINE_UNKNOWN",
                    "当前样式不是可恢复的已知主题，未开始试穿。请先明确恢复原版或应用已知主题。",
                ));
            }
            let mut journal = TrialJournal {
                session: TrialSession {
                    id,
                    name: doc.name.clone(),
                    phase: "preparing".into(),
                    deadline_ms: 0,
                    error: None,
                    synced_sequence: None,
                    css_hash: None,
                },
                candidate,
                draft: Some(doc),
                previous_key,
                previous_intent: load_persistent_state(&app)?,
                committed: false,
                commit_reference: None,
                target_id: Some(baseline_cdp.target_id.clone()),
            };
            set_trial(&app, Some(journal.clone()))?;
            if let Err(error) =
                apply_theme_locked(&app, &journal.candidate.id, false, ApplyOrigin::Manual)
            {
                journal.session.phase = "pendingRecovery".into();
                journal.session.error = Some(error.code.clone());
                set_trial(&app, Some(journal))?;
                let _ = recover_trial_locked(&app);
                return Err(error);
            }
            if studio.close_requested.load(Ordering::SeqCst) {
                recover_trial_locked(&app)?;
                return Err(AppError::new(
                    "TRIAL_CANCELLED",
                    "编辑窗口已关闭，试用已回退。",
                ));
            }
            journal.session = TrialSession::live(
                journal.session.id.clone(),
                journal.session.name.clone(),
                Local::now().timestamp_millis(),
                journal.draft.as_ref().unwrap().edit_sequence,
            );
            let connected = live_preview::Cdp::connect(load_persistent_state(&app)?.preferred_port)
                .and_then(|mut cdp| {
                    cdp.verify(
                        &journal.candidate.runtime_theme_id,
                        &journal.draft.as_ref().unwrap().compiled.css,
                    )?;
                    Ok(cdp.target_id.clone())
                });
            match connected {
                Ok(target) => journal.target_id = Some(target),
                Err(error) => {
                    let _ = recover_trial_locked(&app);
                    return Err(error);
                }
            }
            journal.session.css_hash = Some(live_preview::css_hash(
                &journal.draft.as_ref().unwrap().compiled.css,
            ));
            set_trial(&app, Some(journal.clone()))?;
            Ok(journal.session)
        })
    })
    .await
}

fn restore_intent(app: &AppHandle, before: &PersistentState) -> AppResult<()> {
    let mut saved = load_persistent_state(app)?;
    saved.desired_state = before.desired_state.clone();
    saved.selected_theme_id = before.selected_theme_id.clone();
    saved.selected_theme_version = before.selected_theme_version.clone();
    saved.auto_keep_theme = before.auto_keep_theme;
    save_persistent_state(app, &saved)
}
fn active_journal(app: &AppHandle, id: &str) -> AppResult<TrialJournal> {
    if app
        .state::<StudioState>()
        .close_requested
        .load(Ordering::SeqCst)
    {
        return Err(AppError::new(
            "TRIAL_WINDOW_CLOSED",
            "编辑窗口已关闭，正在恢复之前的效果。",
        ));
    }
    app.state::<StudioState>()
        .trial
        .lock()
        .map_err(|_| lock_error())?
        .clone()
        .filter(|j| {
            j.session.id == id
                && j.session.phase == "active"
                && Local::now().timestamp_millis() < j.session.deadline_ms
        })
        .ok_or_else(|| AppError::new("TRIAL_EXPIRED", "试穿已结束或正在恢复。"))
}
fn trial_cdp(app: &AppHandle, j: &TrialJournal) -> AppResult<live_preview::Cdp> {
    let cdp = live_preview::Cdp::connect(load_persistent_state(app)?.preferred_port)?;
    if j.target_id.as_deref() != Some(cdp.target_id.as_str()) {
        return Err(AppError::new(
            "TRIAL_RENDERER_CHANGED",
            "WorkBuddy 页面已重新加载，请先恢复后重新试穿。",
        ));
    }
    Ok(cdp)
}
#[tauri::command]
pub(crate) async fn studio_renew_trial(
    app: AppHandle,
    trial_id: String,
) -> AppResult<TrialSession> {
    blocking(move || {
        let managed = app.state::<AppState>();
        host_operation(&app, &managed, || {
            let mut j = active_journal(&app, &trial_id)?;
            j.session.renew(Local::now().timestamp_millis())?;
            set_trial(&app, Some(j.clone()))?;
            Ok(j.session)
        })
    })
    .await
}
#[tauri::command]
pub(crate) async fn studio_sync_trial(
    app: AppHandle,
    trial_id: String,
    sequence: u64,
) -> AppResult<TrialSession> {
    blocking(move || {
        let managed = app.state::<AppState>();
        host_operation(&app, &managed, || {
            let mut j = active_journal(&app, &trial_id)?;
            let old = j.draft.clone().ok_or_else(lock_error)?;
            let doc = {
                let studio = app.state::<StudioState>();
                let _lock = studio.store_lock.lock().map_err(|_| lock_error())?;
                ThemeStore::production()?.read_draft(&old.draft_id)?
            };
            if doc.edit_sequence != sequence || sequence < old.edit_sequence {
                return Err(AppError::new(
                    "TRIAL_STALE_UPDATE",
                    "已跳过旧参数，等待最新修改。",
                ));
            }
            if doc.asset_id != old.asset_id {
                return Err(AppError::new(
                    "TRIAL_IMAGE_CHANGED",
                    "试穿期间不能替换背景资源，请先结束试穿。",
                ));
            }
            let result: AppResult<TrialSession> = (|| {
                let mut cdp = trial_cdp(&app, &j)?;
                if j.session.synced_sequence == Some(sequence)
                    && old.compiled.css == doc.compiled.css
                {
                    cdp.verify(&j.candidate.runtime_theme_id, &doc.compiled.css)?;
                    return Ok(j.session.clone());
                }
                // Persist recovery state before the host changes. Original baseline remains immutable.
                j.draft = Some(doc.clone());
                j.session.synced_sequence = None;
                set_trial(&app, Some(j.clone()))?;
                cdp.update(
                    &j.candidate.runtime_theme_id,
                    &old.compiled.css,
                    &doc.compiled.css,
                    sequence,
                )?;
                active_journal(&app, &trial_id)?;
                j.session.synced_sequence = Some(sequence);
                j.session.css_hash = Some(live_preview::css_hash(&doc.compiled.css));
                set_trial(&app, Some(j.clone()))?;
                Ok(j.session.clone())
            })();
            if let Err(ref error) = result {
                j.session.phase = "pendingRecovery".into();
                j.session.error = Some(error.code.clone());
                set_trial(&app, Some(j))?;
                let _ = recover_trial_locked(&app);
            }
            result
        })
    })
    .await
}
#[tauri::command]
pub(crate) async fn studio_capture_trial(
    app: AppHandle,
    trial_id: String,
) -> AppResult<live_preview::RenderedPreview> {
    blocking(move || {
        let studio = app.state::<StudioState>();
        let _capture = studio
            .capture_gate
            .try_lock()
            .map_err(|_| AppError::new("CAPTURE_THROTTLED", "正在获取画面，请稍后刷新。"))?;
        let j = active_journal(&app, &trial_id)?;
        {
            let mut at = studio.capture_at.lock().map_err(|_| lock_error())?;
            if at.is_some_and(|at| at.elapsed() < Duration::from_secs(1)) {
                return Err(AppError::new(
                    "CAPTURE_THROTTLED",
                    "画面更新过于频繁，请稍后刷新。",
                ));
            }
            *at = Some(Instant::now());
        }
        let doc = j.draft.as_ref().ok_or_else(lock_error)?;
        let version = studio
            .snapshot
            .lock()
            .ok()
            .and_then(|s| s.as_ref().and_then(|s| s.workbuddy.version.clone()));
        let mut cdp = trial_cdp(&app, &j)?;
        let path = load_persistent_state(&app)?.workbuddy_path.ok_or_else(|| {
            AppError::new(
                "WORKBUDDY_PATH_REQUIRED",
                "请先在设置中识别 WorkBuddy 路径，再刷新画面。",
            )
        })?;
        capture_window::require_available(&path)?;
        let frame = live_preview::capture(
            &mut cdp,
            &trial_id,
            doc,
            &j.candidate.runtime_theme_id,
            version,
        )?;
        capture_window::require_available(&path)?;
        // A sync, rollback, renderer change or close may finish while CDP is waiting.
        // Validate and publish under the same short trial lock; never publish a late frame.
        let current = studio.trial.lock().map_err(|_| lock_error())?;
        let current = current.as_ref().ok_or_else(capture_changed)?;
        validate_capture_binding(current, &frame.view, Local::now().timestamp_millis())?;
        let view = frame.view.clone();
        let mut stored = studio.capture.lock().map_err(|_| lock_error())?;
        if studio.close_requested.load(Ordering::SeqCst) {
            return Err(capture_changed());
        }
        *stored = Some(frame);
        Ok(view)
    })
    .await
}
fn capture_changed() -> AppError {
    AppError::new(
        "CAPTURE_CHANGED",
        "截图期间参数或试穿状态发生变化，已丢弃旧画面，请刷新。",
    )
}
fn validate_capture_binding(
    j: &TrialJournal,
    frame: &live_preview::RenderedPreview,
    now: i64,
) -> AppResult<()> {
    let doc = j.draft.as_ref().ok_or_else(capture_changed)?;
    if j.session.id != frame.trial_id
        || j.session.phase != "active"
        || now >= j.session.deadline_ms
        || j.session.synced_sequence != Some(frame.sequence)
        || j.session.css_hash.as_deref() != Some(frame.css_hash.as_str())
        || j.target_id.as_deref() != Some(frame.target_id.as_str())
        || doc.draft_id != frame.draft_id
        || doc.edit_sequence != frame.sequence
        || live_preview::css_hash(&doc.compiled.css) != frame.css_hash
    {
        return Err(capture_changed());
    }
    Ok(())
}
#[tauri::command]
pub(crate) async fn studio_export_capture(app: AppHandle, capture_id: String) -> AppResult<bool> {
    blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        let png = app
            .state::<StudioState>()
            .capture
            .lock()
            .map_err(|_| lock_error())?
            .as_ref()
            .filter(|f| f.view.id == capture_id)
            .map(|f| f.png.clone())
            .ok_or_else(|| AppError::new("CAPTURE_EXPIRED", "截图已过期，请刷新后导出。"))?;
        let Some(file) = app
            .dialog()
            .file()
            .set_title("保存真实效果截图（可能包含私人内容，仅保存本机）")
            .add_filter("PNG 图片", &["png"])
            .set_file_name("WorkBuddy-real-preview.png")
            .blocking_save_file()
        else {
            return Ok(false);
        };
        let path = file
            .into_path()
            .map_err(|_| AppError::new("EXPORT_PATH_INVALID", "请选择本机 PNG 文件。"))?;
        if path
            .extension()
            .and_then(|s| s.to_str())
            .is_none_or(|s| !s.eq_ignore_ascii_case("png"))
        {
            return Err(AppError::new(
                "EXPORT_PATH_INVALID",
                "仅允许导出 PNG 图片。",
            ));
        }
        atomic_write(&path, &png)
            .map_err(|_| AppError::new("EXPORT_FAILED", "截图保存失败，请检查目录权限。"))?;
        Ok(true)
    })
    .await
}
fn recover_trial_locked(app: &AppHandle) -> AppResult<()> {
    recover_trial_with_policy(app, false)
}
fn recovery_surface_is_known(
    count: u64,
    id: Option<&str>,
    hash: Option<&str>,
    known: &[(String, String)],
    allow_original: bool,
) -> bool {
    match count {
        0 => allow_original,
        1 => known.iter().any(|(known_id, known_hash)| {
            Some(known_id.as_str()) == id && Some(known_hash.as_str()) == hash
        }),
        _ => false,
    }
}
fn package_fingerprint(app: &AppHandle, key: &str) -> AppResult<(String, String)> {
    let theme = resolve_theme(app, key)?;
    let bytes = fs::read(&theme.package_path).map_err(|_| lock_error())?;
    let package: Value = serde_json::from_slice(&bytes).map_err(|_| lock_error())?;
    let css = package
        .pointer("/targets/workbuddy/css")
        .and_then(Value::as_str)
        .ok_or_else(lock_error)?;
    Ok((theme.record.runtime_theme_id, live_preview::css_hash(css)))
}
fn recover_trial_with_policy(app: &AppHandle, restore_external: bool) -> AppResult<()> {
    let studio = app.state::<StudioState>();
    let Some(mut j) = studio.trial.lock().map_err(|_| lock_error())?.clone() else {
        return Ok(());
    };
    if j.committed {
        return set_trial(app, None);
    }
    j.session.phase = "restoring".into();
    set_trial(app, Some(j.clone()))?;
    let result = (|| {
        let port = load_persistent_state(app)?.preferred_port;
        if !is_workbuddy_running() {
            return Err(AppError::new(
                "TRIAL_WAITING_WORKBUDDY",
                "WorkBuddy 已关闭；恢复记录已保留，待它重新连接后验证恢复结果。",
            ));
        }
        let mut cdp = live_preview::Cdp::connect(port)?;
        let (count, id, hash) = cdp.theme_fingerprint()?;
        // Identity alone is insufficient: another tool can edit/remove our style node.
        // Accept both sides of a journaled CSS update, so a mid-update crash can roll back.
        let mut known = Vec::new();
        if let Some(hash) = &j.session.css_hash {
            known.push((j.candidate.runtime_theme_id.clone(), hash.clone()));
        }
        if let Some(doc) = &j.draft {
            known.push((
                j.candidate.runtime_theme_id.clone(),
                live_preview::css_hash(&doc.compiled.css),
            ));
        }
        if let Some(key) = &j.previous_key {
            known.push(package_fingerprint(app, key)?);
        }
        if let Some(reference) = &j.commit_reference {
            known.push(package_fingerprint(app, &reference.key())?);
        }
        // A new renderer can legitimately start with no style. On the same renderer,
        // removing a themed baseline is ambiguous: stop and ask rather than overwrite.
        let allow_original = j.previous_key.is_none()
            || j.target_id.as_deref().is_some_and(|id| id != cdp.target_id);
        if !restore_external
            && !recovery_surface_is_known(
                count,
                id.as_deref(),
                hash.as_deref(),
                &known,
                allow_original,
            )
        {
            let mut saved = load_persistent_state(app)?;
            saved.auto_keep_theme = false;
            save_persistent_state(app, &saved)?;
            return Err(AppError::new(
                "TRIAL_EXTERNAL_CHANGE",
                "检测到外部换肤，已暂停自动保持；请手动选择保留当前外观或恢复原版。",
            ));
        }
        if let Some(key) = &j.previous_key {
            apply_theme_locked(app, key, false, ApplyOrigin::Manual)?;
        } else {
            restore_with_codedrobe(app, port)?;
            if read_theme_node_metadata(port)?.style_node_count != 0 {
                return Err(AppError::new("RESTORE_VERIFY_FAILED", "原版恢复尚未验证。"));
            }
        }
        restore_intent(app, &j.previous_intent)
    })();
    match result {
        Ok(()) => set_trial(app, None),
        Err(error) => {
            j.session.phase = "pendingRecovery".into();
            j.session.error = Some(error.code.clone());
            set_trial(app, Some(j))?;
            Err(error)
        }
    }
}
#[tauri::command]
pub(crate) async fn studio_cancel_trial(app: AppHandle) -> AppResult<()> {
    blocking(move || {
        let managed = app.state::<AppState>();
        host_operation(&app, &managed, || recover_trial_with_policy(&app, true))
    })
    .await
}
#[tauri::command]
pub(crate) async fn studio_resolve_recovery(
    app: AppHandle,
    restore_original: bool,
) -> AppResult<()> {
    blocking(move || {
        let managed = app.state::<AppState>();
        host_operation(&app, &managed, || {
            let studio = app.state::<StudioState>();
            let external = studio
                .trial
                .lock()
                .map_err(|_| lock_error())?
                .as_ref()
                .is_some_and(|j| j.session.error.as_deref() == Some("TRIAL_EXTERNAL_CHANGE"));
            let corrupt = studio
                .recovery_error
                .lock()
                .map_err(|_| lock_error())?
                .is_some();
            if !external && !corrupt {
                return Err(AppError::new(
                    "RECOVERY_NOT_REQUIRED",
                    "当前没有需要人工处理的恢复异常。",
                ));
            }
            if corrupt && !restore_original {
                return Err(AppError::new(
                    "RECOVERY_TARGET_UNKNOWN",
                    "损坏的记录需要显式恢复原版，不能猜测原主题。",
                ));
            }
            let metadata = read_theme_node_metadata(load_persistent_state(&app)?.preferred_port)?;
            if metadata.style_node_count > 1 {
                return Err(AppError::new(
                    "STYLE_NODE_COUNT_INVALID",
                    "存在多个主题节点，请先检查 WorkBuddy。",
                ));
            }
            if restore_original {
                restore_theme_locked(&app)?;
            }
            // Keep a recoverable copy of the problematic journal before releasing its dependencies.
            let path = journal_path()?;
            if path.exists() {
                let archive = ThemeStore::production()?.root.join("recovery-archive");
                fs::create_dir_all(&archive).map_err(|_| lock_error())?;
                fs::copy(
                    &path,
                    archive.join(format!("{}.json", uuid::Uuid::new_v4().simple())),
                )
                .map_err(|_| lock_error())?;
            }
            persist_original_choice(&app)?;
            set_login_autostart(&app, false)?;
            set_trial(&app, None)?;
            *studio.recovery_error.lock().map_err(|_| lock_error())? = None;
            if let Ok(mut cached) = studio.snapshot.lock() {
                if let Some(ref mut value) = *cached {
                    value.error = None;
                }
            }
            publish_state(&app);
            Ok(())
        })
    })
    .await
}
#[tauri::command]
pub(crate) async fn studio_confirm_trial(app: AppHandle) -> AppResult<ThemeRef> {
    blocking(move || {
        let managed = app.state::<AppState>();
        host_operation(&app, &managed, || {
            let studio = app.state::<StudioState>();
            let mut j = studio
                .trial
                .lock()
                .map_err(|_| lock_error())?
                .clone()
                .ok_or_else(|| AppError::new("TRIAL_EXPIRED", "试用已结束。"))?;
            if j.session.phase != "active"
                || Local::now().timestamp_millis() >= j.session.deadline_ms
            {
                return Err(AppError::new(
                    "TRIAL_EXPIRED",
                    "试用已到期，请恢复后重新试用。",
                ));
            }
            let metadata = read_theme_node_metadata(load_persistent_state(&app)?.preferred_port)?;
            if metadata.style_node_count != 1
                || metadata.runtime_theme_id.as_deref() != Some(&j.candidate.runtime_theme_id)
            {
                return Err(AppError::new(
                    "TRIAL_CHANGED",
                    "实机主题已经变化，不能确认保留。",
                ));
            }
            let doc = j.draft.as_ref().ok_or_else(|| lock_error())?;
            trial_cdp(&app, &j)?.verify(&j.candidate.runtime_theme_id, &doc.compiled.css)?;
            let store = ThemeStore::production()?;
            let reference = {
                let _lock = studio.store_lock.lock().map_err(|_| lock_error())?;
                let current = store.read_draft(&doc.draft_id)?;
                if current.edit_sequence != doc.edit_sequence {
                    return Err(AppError::new(
                        "DRAFT_CHANGED",
                        "草稿在试用期间变化，请恢复后重新试用。",
                    ));
                }
                store.save(&doc.draft_id)?
            };
            // Reapply the immutable saved revision before making it the monitor's persistent intent.
            j.commit_reference = Some(reference.clone());
            j.session.phase = "committing".into();
            set_trial(&app, Some(j.clone()))?;
            apply_theme_locked(&app, &reference.key(), false, ApplyOrigin::Manual)?;
            persist_manual_theme_choice(&app, &reference.key())?;
            j.committed = true;
            set_trial(&app, Some(j))?;
            set_trial(&app, None)?;
            Ok(reference)
        })
    })
    .await
}

#[tauri::command]
pub(crate) async fn studio_runtime(app: AppHandle) -> AppResult<RuntimeSnapshot> {
    if let Some(snapshot) = app
        .state::<StudioState>()
        .snapshot
        .lock()
        .map_err(|_| lock_error())?
        .clone()
    {
        return Ok(snapshot);
    }
    blocking(move || {
        let managed = app.state::<AppState>();
        with_manual_operation(&managed, || collect_snapshot(&app))
    })
    .await
}
fn collect_snapshot(app: &AppHandle) -> AppResult<RuntimeSnapshot> {
    let studio = app.state::<StudioState>();
    let workbuddy = detect_workbuddy_inner(app)?;
    let managed = app.state::<AppState>();
    let result = RuntimeSnapshot {
        workbuddy,
        runtime: runtime_status(app, &managed)?,
        trial: studio
            .trial
            .lock()
            .map_err(|_| lock_error())?
            .as_ref()
            .map(|j| j.session.clone()),
        operation: studio.operation.lock().map_err(|_| lock_error())?.clone(),
        captured_at: now_timestamp(),
        error: studio
            .recovery_error
            .lock()
            .map_err(|_| lock_error())?
            .clone(),
    };
    *studio.snapshot.lock().map_err(|_| lock_error())? = Some(result.clone());
    *studio.sampled_at.lock().map_err(|_| lock_error())? = Some(Instant::now());
    Ok(result)
}
pub(crate) fn known_healthy(app: &AppHandle, key: &str) -> bool {
    let studio = app.state::<StudioState>();
    let fresh = studio
        .sampled_at
        .lock()
        .ok()
        .and_then(|s| *s)
        .is_some_and(|t| t.elapsed() < MONITOR_INTERVAL);
    fresh
        && studio
            .snapshot
            .lock()
            .ok()
            .and_then(|s| s.clone())
            .is_some_and(|s| {
                s.workbuddy.cdp_available
                    && s.workbuddy.style_node_count == Some(1)
                    && s.workbuddy.current_theme_id.as_deref() == Some(key)
            })
}
pub(crate) fn initialize_or_block(app: &AppHandle) {
    if let Err(error) = initialize(app) {
        let studio = app.state::<StudioState>();
        studio.trial_pending.store(true, Ordering::SeqCst);
        if let Ok(mut issue) = studio.recovery_error.lock() {
            *issue = Some(error.message);
        };
    }
}
fn initialize(app: &AppHandle) -> AppResult<()> {
    let path = journal_path()?;
    if path.exists() {
        let mut j: TrialJournal =
            serde_json::from_slice(&fs::read(&path).map_err(|_| lock_error())?).map_err(|_| {
                AppError::new(
                    "TRIAL_RECOVERY_INVALID",
                    "试用恢复记录损坏，已阻止自动应用；请检查数据文件。",
                )
            })?;
        // All persisted candidate paths must still be confined to this specific trial directory.
        if j.session.id.len() != 32
            || !j.session.id.bytes().all(|v| v.is_ascii_hexdigit())
            || PathBuf::from(&j.candidate.package_path)
                != ThemeStore::production()?
                    .root
                    .join("trials")
                    .join(&j.session.id)
                    .join("theme.codedrobe-theme")
        {
            return Err(AppError::new(
                "TRIAL_RECOVERY_INVALID",
                "试用恢复路径无效。",
            ));
        }
        j.session.phase = "pendingRecovery".into();
        set_trial(app, Some(j))?;
    }
    Ok(())
}
pub(crate) fn tick(app: &AppHandle) {
    let studio = app.state::<StudioState>();
    let journal = studio.trial.lock().ok().and_then(|j| j.clone());
    if let Some(j) = journal {
        if j.session.phase != "active" || Local::now().timestamp_millis() >= j.session.deadline_ms {
            let due = studio
                .last_recovery
                .lock()
                .ok()
                .and_then(|t| *t)
                .is_none_or(|t| t.elapsed() >= MONITOR_INTERVAL);
            let managed = app.state::<AppState>();
            if due && j.session.error.as_deref() != Some("TRIAL_EXTERNAL_CHANGE") {
                if let Ok(_lock) = managed.operation_lock.try_lock() {
                    if let Ok(mut last) = studio.last_recovery.lock() {
                        *last = Some(Instant::now());
                    }
                    let _ = recover_trial_locked(app);
                };
            }
        }
    }
    let due = studio
        .last_sample
        .lock()
        .map(|mut last| {
            if last.is_some_and(|t| t.elapsed() < MONITOR_INTERVAL) {
                false
            } else {
                *last = Some(Instant::now());
                true
            }
        })
        .unwrap_or(false);
    if !due {
        return;
    }
    let managed = app.state::<AppState>();
    if let Ok(_lock) = managed.operation_lock.try_lock() {
        match collect_snapshot(app) {
            Ok(snapshot) => {
                let _ = app.emit("studio-runtime", snapshot);
                let live = studio
                    .trial
                    .lock()
                    .ok()
                    .and_then(|j| j.clone())
                    .filter(|j| j.session.phase == "active");
                if let Some(mut j) = live {
                    let check = trial_cdp(app, &j).and_then(|mut cdp| {
                        cdp.verify(
                            &j.candidate.runtime_theme_id,
                            &j.draft.as_ref().ok_or_else(lock_error)?.compiled.css,
                        )
                    });
                    if let Err(error) = check {
                        j.session.phase = "pendingRecovery".into();
                        j.session.error = Some(error.code);
                        let _ = set_trial(app, Some(j));
                        let _ = recover_trial_locked(app);
                    }
                }
            }
            Err(error) => {
                let _ = app.emit("studio-runtime-error", error.code);
            }
        }
    };
}
pub(crate) fn close_window(app: &AppHandle) {
    let studio = app.state::<StudioState>();
    // Mark closed before clearing buffers, so an in-flight capture cannot refill them.
    studio.mark_closed();
    if let Ok(mut capture) = studio.capture.lock() {
        *capture = None;
    }
    if let Ok(mut trial) = studio.trial.lock() {
        if let Some(ref mut j) = *trial {
            j.session.phase = "pendingRecovery".into();
            let _ = persist_trial(j);
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_trial_start_cannot_reopen_after_window_close() {
        let studio = StudioState::default();
        let before_close = studio.start_epoch().unwrap();
        studio.mark_closed();
        assert!(studio.begin_start(before_close).is_err());
        assert!(studio.close_requested.load(Ordering::SeqCst));
        let new_command = studio.start_epoch().unwrap();
        studio.begin_start(new_command).unwrap();
        assert!(!studio.close_requested.load(Ordering::SeqCst));
    }
    #[test]
    fn recovery_never_overwrites_unknown_css_or_same_renderer_external_restore() {
        let known: Vec<(String, String)> = vec![
            ("trial".into(), "old".into()),
            ("trial".into(), "new".into()),
            ("previous".into(), "baseline".into()),
            ("committed".into(), "saved".into()),
        ];
        for (id, hash) in &known {
            assert!(recovery_surface_is_known(
                1,
                Some(id),
                Some(hash),
                &known,
                false
            ));
        }
        assert!(!recovery_surface_is_known(
            1,
            Some("trial"),
            Some("external"),
            &known,
            false
        ));
        assert!(!recovery_surface_is_known(0, None, None, &known, false));
        assert!(recovery_surface_is_known(0, None, None, &known, true));
        assert!(!recovery_surface_is_known(
            2,
            Some("trial"),
            Some("new"),
            &known,
            true
        ));
    }
    #[test]
    fn close_guard_never_reactivates_a_cloned_start_or_renewal() {
        let mut live = TrialSession::live("trial".into(), "fixture".into(), 1000, 0);
        assert!(!close_guard(&mut live, false));
        live.renew(2000).unwrap();
        assert!(close_guard(&mut live, true));
        assert_eq!(live.phase, "pendingRecovery");
        assert_eq!(live.error.as_deref(), Some("TRIAL_WINDOW_CLOSED"));
        assert!(live.renew(3000).is_err());
        assert!(!close_guard(&mut live, true));
    }
    #[test]
    fn late_capture_cannot_publish_after_sync_rollback_or_renderer_change() {
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            64,
            64,
            image::Rgb([120, 180, 160]),
        ))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
        let doc = store
            .import("fixture.png", &png.into_inner())
            .unwrap()
            .document;
        let mut session =
            TrialSession::live("trial".into(), "fixture".into(), 1000, doc.edit_sequence);
        session.css_hash = Some(live_preview::css_hash(&doc.compiled.css));
        let j = TrialJournal {
            session,
            candidate: ThemeRecord {
                id: "trial".into(),
                name: "fixture".into(),
                description: String::new(),
                package_path: String::new(),
                preview_path: String::new(),
                verified_work_buddy_version: "5.2.6".into(),
                theme_version: "1.0.0".into(),
                runtime_theme_id: "fixture".into(),
                is_custom: true,
            },
            draft: Some(doc.clone()),
            previous_key: None,
            previous_intent: PersistentState::default(),
            committed: false,
            commit_reference: None,
            target_id: Some("renderer".into()),
        };
        let frame = live_preview::RenderedPreview {
            id: "frame".into(),
            trial_id: "trial".into(),
            draft_id: doc.draft_id.clone(),
            sequence: doc.edit_sequence,
            css_hash: j.session.css_hash.clone().unwrap(),
            target_id: "renderer".into(),
            width: 64,
            height: 64,
            scene: "home".into(),
            captured_at: String::new(),
            workbuddy_version: Some("5.2.6".into()),
            image_data_url: String::new(),
            capture_ms: 0,
        };
        assert!(validate_capture_binding(&j, &frame, 1001).is_ok());
        let mut changed = j.clone();
        changed.session.synced_sequence = None;
        assert!(validate_capture_binding(&changed, &frame, 1001).is_err());
        let mut changed = j.clone();
        changed.draft.as_mut().unwrap().edit_sequence += 1;
        assert!(validate_capture_binding(&changed, &frame, 1001).is_err());
        let mut changed = j.clone();
        changed.session.phase = "pendingRecovery".into();
        assert!(validate_capture_binding(&changed, &frame, 1001).is_err());
        let mut changed = j.clone();
        changed.target_id = Some("new-renderer".into());
        assert!(validate_capture_binding(&changed, &frame, 1001).is_err());
        let mut changed = j.clone();
        changed.draft.as_mut().unwrap().compiled.css.push(' ');
        assert!(validate_capture_binding(&changed, &frame, 1001).is_err());
        assert!(validate_capture_binding(&j, &frame, j.session.deadline_ms).is_err());
    }
    #[test]
    fn live_trial_renewal_adds_ten_minutes_but_never_revives_expired_session() {
        let mut trial = TrialSession::live("test".into(), "主题".into(), 1000, 7);
        assert_eq!(trial.deadline_ms, 601000);
        trial.renew(2000).unwrap();
        assert_eq!(trial.deadline_ms, 1201000);
        assert!(trial.renew(1201000).is_err());
        assert_eq!(trial.synced_sequence, Some(7));
    }
    #[test]
    fn cancelled_or_overdue_jobs_cannot_publish() {
        let token = AtomicBool::new(false);
        assert!(job_checkpoint(&token, Duration::ZERO, Duration::from_secs(20)).is_ok());
        assert_eq!(
            job_checkpoint(&token, Duration::from_secs(21), Duration::from_secs(20))
                .unwrap_err()
                .code,
            "TASK_TIMEOUT"
        );
        token.store(true, Ordering::SeqCst);
        assert_eq!(
            job_checkpoint(&token, Duration::ZERO, Duration::from_secs(20))
                .unwrap_err()
                .code,
            "TASK_CANCELLED"
        );
    }
    #[test]
    fn references_pin_saved_revisions() {
        let id = "custom-v2-0123456789abcdef0123456789abcdef";
        let old = ThemeRef::parse(&format!("{id}@1")).unwrap();
        let new = ThemeRef::parse(&format!("{id}@2")).unwrap();
        assert_ne!(old.key(), new.key());
        assert_eq!(
            theme_library::reference_from_runtime(&theme_library::runtime_id(&old)),
            Some(old)
        );
        assert!(ThemeRef::parse("custom-v2-../evil@1").is_err());
    }
    #[test]
    fn trial_journal_excludes_secret_and_payload() {
        let j = TrialSession {
            id: "a".into(),
            name: "x".into(),
            phase: "active".into(),
            deadline_ms: 60000,
            error: None,
            synced_sequence: None,
            css_hash: None,
        };
        let body = serde_json::to_string(&j).unwrap();
        assert!(!body.contains("apiKey"));
        assert!(!body.contains("base64"));
    }
}
