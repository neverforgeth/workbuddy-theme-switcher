//! Runtime-only compatibility. Never writes a source package or theme revision.
use super::*;
pub(crate) const VERSION: &str = "workbuddy-164.1";
pub(crate) const DOM: &str =
    include_str!("../../vendor/codedrobe/src/adapters/workbuddy-compat/dom.js");
const PAINT: &str = include_str!("../../vendor/codedrobe/src/adapters/workbuddy-compat/paint.js");
const BUILTIN: &str =
    include_str!("../../vendor/codedrobe/src/adapters/workbuddy-compat/builtin.css");
const IMAGE: &str = include_str!("../../vendor/codedrobe/src/adapters/workbuddy-compat/image.css");
const COMPONENTS: &str =
    include_str!("../../vendor/codedrobe/src/adapters/workbuddy-compat/components.css");
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EffectiveTheme {
    pub original_hash: String,
    pub adapter_version: String,
    pub structure: String,
    pub css: String,
    pub final_hash: String,
}
pub(crate) fn compile(css: &str, structure: &str) -> AppResult<EffectiveTheme> {
    let mut final_css = css.to_string();
    if structure == "cr-v1" {
        let aliases = if css.contains("--fusion-main-bg:") {
            IMAGE
        } else if css.contains("--wb-theme-surface-composer-rgb:") {
            BUILTIN
        } else {
            return Err(AppError::new(
                "COMPAT_PALETTE_UNSUPPORTED",
                "此旧主题缺少已知配色契约，原文件未修改；请使用可适配主题或创建新版副本。",
            ));
        };
        final_css += &format!("\n/* {VERSION} */\n{aliases}\n{COMPONENTS}");
    } else if structure != "legacy" {
        return Err(AppError::new(
            "COMPAT_STRUCTURE_UNSUPPORTED",
            "当前页面结构尚未识别，未修改主题。",
        ));
    }
    Ok(EffectiveTheme {
        original_hash: live_preview::css_hash(css),
        adapter_version: VERSION.into(),
        structure: structure.into(),
        final_hash: live_preview::css_hash(&final_css),
        css: final_css,
    })
}
pub(crate) fn paint_expression() -> String {
    PAINT.replace("__WORKBUDDY_DOM__", DOM)
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ApplyJournal {
    target_id: String,
    previous: Option<Value>,
    candidate_id: String,
    candidate_hash: String,
    #[serde(default)]
    original_error: Option<String>,
}
pub(crate) fn record_apply_failure(app: &AppHandle, journal: &mut ApplyJournal, code: &str) -> AppResult<()> {
    // Only a static code is retained, never subprocess output or user content.
    if journal.original_error.is_none() {
        journal.original_error = Some(code.to_string());
    }
    atomic_write(&journal_path(app)?, &serde_json::to_vec(journal).unwrap())
        .map_err(|_| AppError::new("APPLY_JOURNAL_FAILED", "无法记录应用失败原因；原恢复记录仍保留。"))
}
fn journal_path(_app: &AppHandle) -> AppResult<PathBuf> {
    Ok(runtime_root()?.join("apply-recovery.json"))
}
pub(crate) fn finish_apply(app: &AppHandle) -> AppResult<()> {
    let path = journal_path(app)?;
    if path.exists() {
        fs::remove_file(path)
            .map_err(|_| AppError::new("APPLY_JOURNAL_FAILED", "无法清除已验证的应用恢复记录。"))?;
    }
    Ok(())
}
pub(crate) fn begin_apply(
    app: &AppHandle,
    cdp: &mut live_preview::Cdp,
    theme: &ResolvedTheme,
    effective: &EffectiveTheme,
) -> AppResult<ApplyJournal> {
    if journal_path(app)?.exists() {
        return Err(AppError::new(
            "APPLY_RECOVERY_PENDING",
            "之前的应用尚待恢复，请先恢复后再换肤。",
        ));
    }
    let (count, id, hash) = cdp.theme_fingerprint()?;
    let previous = if count == 0 {
        None
    } else if count == 1 {
        let key = id
            .as_deref()
            .and_then(|id| runtime_key(app, id))
            .ok_or_else(|| {
                AppError::new(
                    "APPLY_BASELINE_UNKNOWN",
                    "当前主题无法识别，未覆盖外部样式。",
                )
            })?;
        let original = resolve_theme(app, &key)?;
        let mut package: Value = serde_json::from_slice(
            &fs::read(&original.package_path)
                .map_err(|_| AppError::new("THEME_READ_FAILED", "无法读取原主题。"))?,
        )
        .map_err(|_| AppError::new("THEME_INVALID", "原主题无效。"))?;
        let source = package["targets"]["workbuddy"]["css"]
            .as_str()
            .unwrap_or("");
        let candidates = [
            Some(source.to_string()),
            compile(source, &effective.structure).ok().map(|e| e.css),
            workbuddy_session::trial_css_for_key(app, &key),
        ];
        let css = candidates
            .into_iter()
            .flatten()
            .find(|css| Some(live_preview::css_hash(css)).as_ref() == hash.as_ref());
        if id.as_deref() != Some(&original.record.runtime_theme_id) || css.is_none() {
            return Err(AppError::new(
                "APPLY_BASELINE_UNKNOWN",
                "原主题样式已被外部修改，未覆盖。",
            ));
        }
        package["targets"]["workbuddy"]["css"] = json!(css.unwrap());
        Some(package)
    } else {
        return Err(AppError::new(
            "STYLE_NODE_COUNT_INVALID",
            "主题节点重复，未覆盖。",
        ));
    };
    let journal = ApplyJournal {
        target_id: cdp.target_id.clone(),
        previous,
        candidate_id: theme.record.runtime_theme_id.clone(),
        candidate_hash: effective.final_hash.clone(),
        original_error: None,
    };
    atomic_write(&journal_path(app)?, &serde_json::to_vec(&journal).unwrap())
        .map_err(|_| AppError::new("APPLY_JOURNAL_FAILED", "无法记录应用前状态，未开始换肤。"))?;
    Ok(journal)
}
pub(crate) fn rollback_apply(app: &AppHandle, journal: &ApplyJournal, port: u16) -> AppResult<()> {
    let mut cdp = live_preview::Cdp::connect_target(port, &journal.target_id)?;
    let (count, id, hash) = cdp.theme_fingerprint()?;
    let previous_css = journal
        .previous
        .as_ref()
        .and_then(|p| p.pointer("/targets/workbuddy/css"))
        .and_then(Value::as_str);
    let previous_id = journal
        .previous
        .as_ref()
        .and_then(|p| p.pointer("/theme/id"))
        .and_then(Value::as_str);
    let previous_matches = count == 1
        && id.as_deref() == previous_id
        && hash == previous_css.map(live_preview::css_hash);
    if !previous_matches
        && !(count == 0
            || (count == 1
                && id.as_deref() == Some(&journal.candidate_id)
                && hash.as_deref() == Some(&journal.candidate_hash)))
    {
        return Err(AppError::new(
            "APPLY_EXTERNAL_CHANGE",
            "恢复期间检测到外部样式变化，恢复记录已保留。",
        ));
    }
    if !previous_matches {
        if let Some(package) = &journal.previous {
            let path = runtime_root()?.join("rollback.codedrobe-theme");
            atomic_write(&path, &serde_json::to_vec(package).unwrap())
                .map_err(|_| AppError::new("APPLY_JOURNAL_FAILED", "回退缓存写入失败。"))?;
            run_codedrobe_target(
                app,
                &[
                    "apply".into(),
                    "--app".into(),
                    "workbuddy".into(),
                    "--theme".into(),
                    child_process_path_string(&path),
                    "--port".into(),
                    port.to_string(),
                    "--no-launch".into(),
                    "--restore-baseline".into(),
                    "--json".into(),
                ],
                Some(&journal.target_id),
            )?;
        } else {
            restore_with_codedrobe_target(app, port, &journal.target_id)?;
        }
    }
    let (count, id, hash) = cdp.theme_fingerprint()?;
    let restored = if let Some(css) = previous_css {
        count == 1 && id.as_deref() == previous_id && hash == Some(live_preview::css_hash(css))
    } else {
        count == 0
    };
    if !restored {
        return Err(AppError::new(
            "RESTORE_VERIFY_FAILED",
            "操作前效果尚未恢复，记录已保留。",
        ));
    }
    finish_apply(app)
}
pub(crate) fn pending(app: &AppHandle) -> bool {
    journal_path(app).is_ok_and(|p| p.exists())
}
pub(crate) fn runtime_key(app: &AppHandle, id: &str) -> Option<String> {
    workbuddy_session::trial_key_for_runtime(app, id)
        .or_else(|| {
            theme_catalog(app)
                .ok()?
                .into_iter()
                .find(|t| t.runtime_theme_id == id)
                .map(|t| t.id)
        })
        .or_else(|| theme_library::reference_from_runtime(id).map(|r| r.key()))
}
pub(crate) fn observe(app: &AppHandle) -> AppResult<Value> {
    let start = Instant::now();
    let port = load_persistent_state(app)?.preferred_port;
    let mut cdp = if let Some(id) = workbuddy_session::pinned_target(app) {
        live_preview::Cdp::connect_target(port, &id)?
    } else {
        live_preview::Cdp::connect(port)?
    };
    let mut report = cdp.coverage()?;
    let (count, id, hash) = cdp.theme_fingerprint()?;
    let mut migration_required = false;
    let mut identity_error = None;
    let identity = if count == 1 {
        if let Some(key) = id.as_deref().and_then(|id| runtime_key(app, id)) {
            if let Some(css) = workbuddy_session::trial_css_for_key(app, &key) {
                hash == Some(live_preview::css_hash(&css))
            } else {
                let original = resolve_theme(app, &key)?;
                match prepare(
                    app,
                    &original,
                    report["structure"].as_str().unwrap_or("unknown"),
                ) {
                    Ok((_, effective)) => {
                        migration_required = effective.original_hash != effective.final_hash
                            && hash.as_deref() == Some(&effective.original_hash);
                        hash == Some(effective.final_hash)
                    }
                    Err(error) => {
                        identity_error = Some(error.code);
                        false
                    }
                }
            }
        } else {
            false
        }
    } else {
        false
    };
    report["identity"] = json!(identity);
    report["migrationRequired"] = json!(migration_required);
    report["durationMs"] = json!(start.elapsed().as_millis());
    if !identity {
        report["status"] = json!("failed");
        report["code"] = json!(identity_error
            .as_deref()
            .unwrap_or("COMPAT_IDENTITY_MISMATCH"));
    }
    if count == 0 {
        report["status"] = json!("unverified");
        report["code"] = json!("NATIVE_NO_THEME");
    }
    Ok(report)
}
#[tauri::command]
pub(crate) async fn studio_export_diagnostic(app: AppHandle) -> AppResult<String> {
    tauri::async_runtime::spawn_blocking(move||{
        // Fixed allowlist document: no status paths, target IDs, CSS, screenshots or account data.
        let report=observe(&app).unwrap_or_else(|e|json!({"adapterVersion":VERSION,"structure":"unknown","scene":"unknown","status":"unverified","code":e.code,"identity":false,"checks":[],"unchecked":["all"],"checkedAt":now_timestamp()}));
        let version=resolve_workbuddy_path(&app)?.as_deref().and_then(read_workbuddy_version);
        let body=json!({"schemaVersion":1,"applicationVersion":env!("CARGO_PKG_VERSION"),"workbuddyVersion":version,"compatibility":report});
        let file=log_directory()?.join(format!("compatibility-{}.json",Local::now().format("%Y%m%d-%H%M%S")));
        atomic_write(&file,&serde_json::to_vec_pretty(&body).unwrap()).map_err(|_|AppError::new("DIAGNOSTIC_WRITE_FAILED","无法导出诊断报告。"))?;
        Ok(file.to_string_lossy().into_owned())
    }).await.map_err(|_|AppError::new("DIAGNOSTIC_FAILED","诊断任务未完成。"))?
}
pub(crate) fn recover_pending(app: &AppHandle) -> AppResult<()> {
    let path = journal_path(app)?;
    if !path.exists() {
        return Ok(());
    }
    let journal: ApplyJournal = serde_json::from_slice(
        &fs::read(path)
            .map_err(|_| AppError::new("APPLY_JOURNAL_FAILED", "无法读取应用恢复记录。"))?,
    )
    .map_err(|_| AppError::new("APPLY_JOURNAL_FAILED", "应用恢复记录无效，已暂停。"))?;
    rollback_apply(app, &journal, load_persistent_state(app)?.preferred_port)
}
pub(crate) fn prepare(
    _app: &AppHandle,
    theme: &ResolvedTheme,
    structure: &str,
) -> AppResult<(ResolvedTheme, EffectiveTheme)> {
    let bytes = fs::read(&theme.package_path)
        .map_err(|_| AppError::new("THEME_READ_FAILED", "无法读取主题包。"))?;
    let mut package: Value = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::new("THEME_INVALID", "主题包无效。"))?;
    let css = package
        .pointer("/targets/workbuddy/css")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::new("THEME_INVALID", "缺少主题样式。"))?;
    let mut effective = compile(css, structure)?;
    if let Some(previous) = workbuddy_session::restoring_css(_app, &theme.record.id) {
        effective.css = previous;
        effective.final_hash = live_preview::css_hash(&effective.css);
    }
    package["targets"]["workbuddy"]["css"] = json!(effective.css);
    // Includes the complete source package so equal CSS with different images cannot collide.
    let cache_key = live_preview::css_hash(&format!(
        "{}:{}:{}",
        String::from_utf8_lossy(&bytes),
        effective.final_hash,
        VERSION
    ));
    let folder = runtime_root()?.join("effective-themes");
    fs::create_dir_all(&folder)
        .map_err(|_| AppError::new("COMPAT_CACHE_WRITE_FAILED", "无法创建运行时主题缓存。"))?;
    let path = folder.join(format!("{cache_key}.codedrobe-theme"));
    let encoded = serde_json::to_vec(&package)
        .map_err(|_| AppError::new("THEME_INVALID", "主题编码失败。"))?;
    if fs::read(&path).ok().as_deref() != Some(encoded.as_slice()) {
        atomic_write(&path, &encoded)
            .map_err(|_| AppError::new("COMPAT_CACHE_WRITE_FAILED", "无法写入运行时主题缓存。"))?;
    }
    Ok((
        ResolvedTheme {
            record: theme.record.clone(),
            package_path: path,
        },
        effective,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_apply_journals_remain_readable_without_a_failure_code() {
        let old = json!({"target_id":"fixed-target","previous":null,"candidate_id":"theme","candidate_hash":"hash"});
        let mut journal: ApplyJournal = serde_json::from_value(old).unwrap();
        assert!(journal.original_error.is_none());
        journal.original_error = Some("COMPAT_IMAGE_INVALID".into());
        let encoded = serde_json::to_vec(&journal).unwrap();
        let restored: ApplyJournal = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored.original_error.as_deref(), Some("COMPAT_IMAGE_INVALID"));
        assert_eq!(restored.target_id, "fixed-target");
    }
    #[test]
    fn legacy_is_byte_identical_and_unknown_palettes_fail_closed() {
        let source = "/* old */\r\nbody {color:red}";
        let result = compile(source, "legacy").unwrap();
        assert_eq!(result.css, source);
        assert_eq!(result.final_hash, result.original_hash);
        assert!(compile(source, "cr-v1").is_err());
        assert!(compile(source, "unknown").is_err());
    }
    #[test]
    fn independent_palette_contracts_and_deterministic_identity() {
        let a = compile("--fusion-main-bg:red;", "cr-v1").unwrap();
        let b = compile("--wb-theme-surface-composer-rgb:1 2 3;", "cr-v1").unwrap();
        assert!(!a.css.contains("--cg-"));
        assert!(!b.css.contains("--fusion-"));
        assert_eq!(
            a.final_hash,
            compile("--fusion-main-bg:red;", "cr-v1")
                .unwrap()
                .final_hash
        );
        assert_ne!(a.final_hash, a.original_hash);
        assert!(a.css.contains(".conversation-shell"));
    }
}
