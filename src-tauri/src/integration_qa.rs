//! Explicit debug-build acceptance harness. Never compiled into release installers.
//! Uses a separate data root and never enables autostart or restarts WorkBuddy.
use super::*;
use workbuddy_session::*;

fn qa_error(message: &str) -> AppError {
    AppError::new("QA_ASSERTION", message)
}
fn record(step: &str, detail: Value) -> AppResult<()> {
    let path = runtime_root()?.join("acceptance.jsonl");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|_| qa_error("report write"))?;
    writeln!(
        file,
        "{}",
        json!({"step":step,"time":now_timestamp(),"detail":detail})
    )
    .map_err(|_| qa_error("report write"))
}
fn assert_theme(app: &AppHandle, expected: Option<&str>) -> AppResult<()> {
    let node = read_theme_node_metadata(load_persistent_state(app)?.preferred_port)?;
    let runtime = expected
        .map(|key| resolve_theme(app, key).map(|t| t.record.runtime_theme_id))
        .transpose()?;
    if node.style_node_count != if expected.is_some() { 1 } else { 0 }
        || node.runtime_theme_id != runtime
    {
        return Err(qa_error("theme metadata mismatch"));
    }
    Ok(())
}
async fn wait_recovery(app: &AppHandle, limit: u64) -> AppResult<()> {
    let started = Instant::now();
    while has_trial(app) && started.elapsed() < Duration::from_secs(limit) {
        std::thread::sleep(Duration::from_millis(250));
    }
    if has_trial(app) {
        return Err(qa_error("recovery timeout"));
    }
    Ok(())
}
fn fixture_bytes(color: [u8; 3]) -> Vec<u8> {
    let mut out = io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(640, 360, |x, y| {
        image::Rgb(color.map(|c| c.saturating_add(((x + y) % 35) as u8)))
    }))
    .write_to(&mut out, image::ImageFormat::Png)
    .unwrap();
    out.into_inner()
}
async fn capture_home(
    app: AppHandle,
    trial_id: String,
) -> AppResult<live_preview::RenderedPreview> {
    let mut cdp = live_preview::Cdp::connect(load_persistent_state(&app)?.preferred_port)?;
    if cdp.evaluate("!!document.querySelector('.wb-home-page')")? != Value::Bool(true) {
        return Err(qa_error("home page required; no screenshot taken"));
    }
    drop(cdp);
    let frame = studio_capture_trial(app, trial_id).await?;
    if frame.scene != "home" {
        return Err(qa_error("scene changed; screenshot discarded"));
    }
    Ok(frame)
}
async fn exercise(app: &AppHandle, mode: &str) -> AppResult<()> {
    let baseline_path = runtime_root()?.join("baseline.json");
    if mode == "recover" {
        wait_recovery(app, 45).await?;
        let key: Option<String> = serde_json::from_slice(
            &fs::read(&baseline_path).map_err(|_| qa_error("baseline absent"))?,
        )
        .map_err(|_| qa_error("baseline invalid"))?;
        assert_theme(app, key.as_deref())?;
        return record("crash-recovery-pass", json!({"restored":key}));
    }
    let before = studio_runtime(app.clone()).await?.workbuddy;
    if matches!(mode, "capture" | "live") {
        let mut cdp = live_preview::Cdp::connect(load_persistent_state(app)?.preferred_port)?;
        if cdp.evaluate("!!document.querySelector('.wb-home-page')")? != Value::Bool(true) {
            return Err(qa_error(
                "home page required before local-only screenshot acceptance",
            ));
        }
    }
    if !before.cdp_available || !matches!(before.style_node_count, Some(0) | Some(1)) {
        return Err(qa_error("connected baseline required; no restart allowed"));
    }
    let baseline = before.current_theme_id;
    assert_theme(app, baseline.as_deref())?;
    atomic_write(&baseline_path, &serde_json::to_vec(&baseline).unwrap())
        .map_err(|_| qa_error("baseline persist"))?;
    record(
        "baseline",
        json!({"theme":baseline,"version":before.version,"nodes":before.style_node_count}),
    )?;
    let view = studio_import(
        app.clone(),
        "qa-landscape.png".into(),
        base64_encode(&fixture_bytes([92, 128, 119])),
        None,
    )
    .await?;
    let id = view.document.draft_id.clone();
    let mut timings = Vec::new();
    for sequence in 1..=40 {
        let started = Instant::now();
        studio_update_draft(
            app.clone(),
            id.clone(),
            theme_library::DraftUpdate {
                name: "QA cache timing".into(),
                controls: ThemeControls {
                    brightness: (sequence % 20) as i16,
                    ..ThemeControls::default()
                },
                sequence,
                ..Default::default()
            },
        )
        .await?;
        timings.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    timings.sort_by(f64::total_cmp);
    record(
        "cached-update-backend-timing",
        json!({"samples":40,"p95Ms":timings[37],"scope":"command dispatch + compile + atomic draft persistence; excludes UI/IPC paint"}),
    )?;
    let result: AppResult<()> = async {
        if mode == "capture" {
            let trial = studio_start_trial(app.clone(), id.clone(), false).await?;
            let app_copy = app.clone();
            let trial_copy = trial.id.clone();
            let capture = tauri::async_runtime::spawn(async move { capture_home(app_copy, trial_copy).await });
            std::thread::sleep(Duration::from_millis(30));
            let started = Instant::now();
            studio_update_draft(app.clone(), id.clone(), theme_library::DraftUpdate {
                name: "QA capture race".into(), controls: ThemeControls {brightness:8, ..ThemeControls::default()}, sequence:41, ..Default::default()
            }).await?;
            studio_sync_trial(app.clone(), trial.id.clone(), 41).await?;
            let elapsed = started.elapsed().as_millis();
            let captured = capture.await.map_err(|_| qa_error("capture task failed"))?;
            record("capture-sync-race",json!({"syncMs":elapsed,"captureResult":captured.as_ref().err().map(|e|e.code.clone()),"returnedSequence":captured.as_ref().ok().map(|v|v.sequence)}))?;
            if elapsed > 1000 { return Err(qa_error("capture blocks CSS sync")); }
            studio_cancel_trial(app.clone()).await?;
            assert_theme(app, baseline.as_deref())?;
            let mut fixtures = vec![("light".to_string(), fixture_bytes([235,239,241])), ("dark".into(),fixture_bytes([8,16,25])), ("saturated".into(),fixture_bytes([231,25,98]))];
            for (name, relative) in [("portrait", "themes/pink-lijiajia/theme.codedrobe-theme"), ("landscape", "themes/oriental-landscape/theme.codedrobe-theme")] {
                let path = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(relative);
                let package: Value = serde_json::from_slice(&fs::read(path).map_err(|_|qa_error("fixture package"))?).map_err(|_|qa_error("fixture JSON"))?;
                let asset=&package["assets"]["images"]["hero"];
                fixtures.push((name.into(),base64_decode(asset["base64"].as_str().ok_or_else(||qa_error("fixture image"))?).ok_or_else(||qa_error("fixture base64"))?));
            }
            for (name, data) in fixtures {
                let ext = match image::guess_format(&data).map_err(|_|qa_error("fixture format"))? { image::ImageFormat::Jpeg=>"jpg", image::ImageFormat::WebP=>"webp", _=>"png" };
                let view=studio_import(app.clone(),format!("{name}.{ext}"),base64_encode(&data),None).await?;
                let trial=studio_start_trial(app.clone(),view.document.draft_id.clone(),false).await?;
                std::thread::sleep(Duration::from_millis(500));
                let frame=capture_home(app.clone(),trial.id.clone()).await?;
                let png=base64_decode(frame.image_data_url.split_once(',').unwrap().1).ok_or_else(||qa_error("capture base64"))?;
                atomic_write(&runtime_root()?.join(format!("actual-{name}.png")), &png).map_err(|_|qa_error("capture artifact"))?;
                record("actual-capture-pass",json!({"fixture":name,"width":frame.width,"height":frame.height,"sequence":frame.sequence,"captureMs":frame.capture_ms,"version":frame.workbuddy_version}))?;
                studio_cancel_trial(app.clone()).await?;
                assert_theme(app,baseline.as_deref())?;
                record("capture-and-rollback-pass",json!({"fixture":name,"localOnly":true}))?;
            }
            return Ok(());
        }
        if matches!(mode,"live"|"lifecycle") {
            let trial=studio_start_trial(app.clone(),id.clone(),false).await?;
            let renewed=studio_renew_trial(app.clone(),trial.id.clone()).await?;
            if renewed.deadline_ms-trial.deadline_ms!=600_000{return Err(qa_error("renewal incorrect"));}
            let mut sync_times=Vec::new();
            for sequence in 41..=80 {
                let started=Instant::now();
                studio_update_draft(app.clone(),id.clone(),theme_library::DraftUpdate{name:"QA 1.5 regional live".into(),controls:ThemeControls{brightness:(sequence%20) as i16,..ThemeControls::default()},sequence,..Default::default()}).await?;
                let synced=studio_sync_trial(app.clone(),trial.id.clone(),sequence).await?;
                if synced.synced_sequence!=Some(sequence){return Err(qa_error("wrong sync sequence"));}
                sync_times.push(started.elapsed().as_secs_f64()*1000.0);
            }
            sync_times.sort_by(f64::total_cmp);
            record("live-update-timing",json!({"samples":40,"p95Ms":sync_times[37],"scope":"backend draft update + host CSS update + verification, excludes UI debounce"}))?;
            if mode=="live" {
            std::thread::sleep(Duration::from_millis(1200));
            let frame=capture_home(app.clone(),trial.id.clone()).await?;
            if frame.sequence!=80{return Err(qa_error("capture sequence"));}
            let png=base64_decode(frame.image_data_url.split_once(',').ok_or_else(||qa_error("capture payload"))?.1).ok_or_else(||qa_error("capture decode"))?;
            atomic_write(&runtime_root()?.join("actual-live.png"),&png).map_err(|_|qa_error("capture artifact"))?;
            record("actual-capture-pass",json!({"width":frame.width,"height":frame.height,"sequence":frame.sequence,"captureMs":frame.capture_ms,"version":frame.workbuddy_version}))?;
            }
            let reference=studio_save_draft(app.clone(),id.clone()).await?;
            assert_theme(app,Some(&format!("trial-{}",trial.id)))?;
            let kept=studio_confirm_trial(app.clone()).await?;
            if kept!=reference{return Err(qa_error("confirmation created unexpected revision"));}
            assert_theme(app,Some(&kept.key()))?;
            record("live-save-confirm-pass",json!({"revision":kept.revision}))?;
            studio_start_trial(app.clone(),id.clone(),false).await?;
            close_window(app);wait_recovery(app,30).await?;assert_theme(app,Some(&kept.key()))?;
            record("live-window-close-recovery-pass",json!({}))?;
            studio_start_trial(app.clone(),id.clone(),false).await?;
            let port=load_persistent_state(app)?.preferred_port;
            {let state=app.state::<AppState>();with_manual_operation(&state,||{let mut s=load_persistent_state(app)?;s.preferred_port=19337;save_persistent_state(app,&s)})?;}
            if studio_cancel_trial(app.clone()).await.is_ok()||!has_trial(app){return Err(qa_error("disconnection falsely reported recovery"));}
            record("live-disconnect-pending-pass",json!({"onlyTestConfigurationChanged":true}))?;
            {let state=app.state::<AppState>();with_manual_operation(&state,||{let mut s=load_persistent_state(app)?;s.preferred_port=port;save_persistent_state(app,&s)})?;}
            wait_recovery(app,30).await?;assert_theme(app,Some(&kept.key()))?;
            record("live-reconnect-recovery-pass",json!({}))?;
            let trial=studio_start_trial(app.clone(),id.clone(),false).await?;
            record("live-ten-minute-timeout-started",json!({"deadline":trial.deadline_ms}))?;
            wait_recovery(app,620).await?;assert_theme(app,Some(&kept.key()))?;
            record("live-ten-minute-timeout-pass",json!({}))?;
            studio_restore(app.clone()).await?;assert_theme(app,None)?;
            record("live-restore-original-pass",json!({"nodes":0}))?;
            return Ok(());
        }
        // A legacy-format package coexists with immutable revisions and stays read-only.
        let legacy_dir = runtime_root()?
            .join("custom-themes")
            .join("custom-qa-legacy");
        fs::create_dir_all(&legacy_dir).map_err(|_| qa_error("legacy fixture directory"))?;
        let preview_path = legacy_dir.join("preview.png");
        atomic_write(&preview_path, &fixture_bytes([92, 128, 119]))
            .map_err(|_| qa_error("legacy fixture"))?;
        let package_path = legacy_dir.join("theme.codedrobe-theme");
        let store = theme_library::ThemeStore::production()?;
        let legacy_doc = store.read_draft(&id)?;
        let hero = fs::read(store.asset_file(&legacy_doc.asset_id, "hero.jpg")?)
            .map_err(|_| qa_error("legacy hero"))?;
        let legacy = ThemeRecord {
            id: "custom-qa-legacy".into(),
            name: "QA old custom theme".into(),
            description: "Legacy fixture".into(),
            package_path: package_path.to_string_lossy().into(),
            preview_path: preview_path.to_string_lossy().into(),
            verified_work_buddy_version: "5.2.6".into(),
            theme_version: "1.0.0".into(),
            runtime_theme_id: "workbuddy-custom-qa-legacy".into(),
            is_custom: true,
        };
        atomic_write(
            &package_path,
            &serde_json::to_vec(&theme_engine::package(
                &legacy.runtime_theme_id,
                &legacy.name,
                &legacy_doc.compiled,
                &hero,
            ))
            .unwrap(),
        )
        .map_err(|_| qa_error("legacy package"))?;
        atomic_write(
            &legacy_dir.join("theme.json"),
            &serde_json::to_vec(&legacy).unwrap(),
        )
        .map_err(|_| qa_error("legacy metadata"))?;
        let library = studio_library(app.clone()).await?;
        if !library.iter().any(|t| !t.is_custom)
            || !library
                .iter()
                .any(|t| t.reference.id == legacy.id && !t.editable)
        {
            return Err(qa_error("mixed legacy library"));
        }
        let reference = theme_library::ThemeRef {
            id: legacy.id.clone(),
            revision: None,
        };
        let copy = studio_copy_legacy(app.clone(), reference.clone()).await?;
        if copy.document.theme_id.is_some() {
            return Err(qa_error("legacy copy overwrites original"));
        }
        let intent = load_persistent_state(app)?;
        studio_apply(app.clone(), reference, false).await?;
        assert_theme(app, Some(&legacy.id))?;
        {
            let state = app.state::<AppState>();
            with_manual_operation(&state, || {
                if let Some(ref key) = baseline {
                    apply_theme_locked(app, key, false, ApplyOrigin::Manual)?;
                } else {
                    restore_theme_locked(app)?;
                }
                save_persistent_state(app, &intent)
            })?;
        }
        record("legacy-mixed-library-copy-apply-pass", json!({}))?;
        let trial = studio_start_trial(app.clone(), id.clone(), false).await?;
        assert_theme(app, Some(&format!("trial-{}", trial.id)))?;
        if mode == "crash" {
            studio_update_draft(app.clone(),id.clone(),theme_library::DraftUpdate{name:"QA crash after live CSS".into(),controls:ThemeControls{brightness:12,..ThemeControls::default()},sequence:41,..Default::default()}).await?;
            studio_sync_trial(app.clone(),trial.id.clone(),41).await?;
            record("crash-after-durable-trial", json!({"trial":trial.id}))?;
            std::process::exit(71);
        }
        studio_cancel_trial(app.clone()).await?;
        assert_theme(app, baseline.as_deref())?;
        record("explicit-cancel-pass", json!({}))?;

        studio_start_trial(app.clone(), id.clone(), false).await?;
        close_window(app);
        wait_recovery(app, 30).await?;
        assert_theme(app, baseline.as_deref())?;
        record("window-close-pass", json!({}))?;

        let trial = studio_start_trial(app.clone(), id.clone(), false).await?;
        record("timeout-started", json!({"deadline":trial.deadline_ms}))?;
        wait_recovery(app, 620).await?;
        assert_theme(app, baseline.as_deref())?;
        record("ten-minute-timeout-pass", json!({}))?;

        studio_start_trial(app.clone(), id.clone(), false).await?;
        let port = load_persistent_state(app)?.preferred_port;
        {
            let state = app.state::<AppState>();
            with_manual_operation(&state, || {
                let mut s = load_persistent_state(app)?;
                s.preferred_port = 19337;
                save_persistent_state(app, &s)
            })?;
        }
        let failure = studio_cancel_trial(app.clone()).await;
        if failure.is_ok() || !has_trial(app) {
            return Err(qa_error("disconnection reported false success"));
        }
        record("disconnect-pending-pass", json!({}))?;
        {
            let state = app.state::<AppState>();
            with_manual_operation(&state, || {
                let mut s = load_persistent_state(app)?;
                s.preferred_port = port;
                save_persistent_state(app, &s)
            })?;
        }
        wait_recovery(app, 30).await?;
        assert_theme(app, baseline.as_deref())?;
        record("reconnect-restore-pass", json!({}))?;

        studio_start_trial(app.clone(), id.clone(), false).await?;
        let reference = studio_confirm_trial(app.clone()).await?;
        assert_theme(app, Some(&reference.key()))?;
        let opened = studio_open_theme(app.clone(), reference.clone()).await?;
        let next = studio_update_draft(
            app.clone(),
            opened.document.draft_id.clone(),
            theme_library::DraftUpdate {
                name: "QA immutable revision".into(),
                controls: ThemeControls {
                    brightness: 15,
                    ..ThemeControls::default()
                },
                sequence: opened.document.edit_sequence + 1,
                ..Default::default()
            },
        )
        .await?;
        let revised = studio_save_draft(app.clone(), next.document.draft_id).await?;
        std::thread::sleep(Duration::from_secs(6));
        assert_theme(app, Some(&reference.key()))?;
        record(
            "confirm-pinned-revision-pass",
            json!({"active":reference,"saved":revised}),
        )?;
        if studio_delete_theme(app.clone(), reference).await.is_ok() {
            return Err(qa_error("active theme deletion allowed"));
        }
        studio_restore(app.clone()).await?;
        assert_theme(app, None)?;
        record("restore-original-zero-nodes-pass", json!({}))?;
        Ok(())
    }
    .await;
    // Always restore the recorded baseline before reporting a failed assertion.
    let _ = studio_cancel_trial(app.clone()).await;
    let restored = {
        let state = app.state::<AppState>();
        with_manual_operation(&state, || {
            let mut intent = load_persistent_state(app)?;
            intent.preferred_port = DEFAULT_CDP_PORT;
            intent.auto_keep_theme = false;
            save_persistent_state(app, &intent)?;
            if let Some(key) = &baseline {
                apply_theme_locked(app, key, false, ApplyOrigin::Manual)?;
            } else {
                restore_theme_locked(app)?;
            }
            assert_theme(app, baseline.as_deref())
        })
    };
    record(
        "baseline-restoration",
        json!({"success":restored.is_ok(),"theme":baseline}),
    )?;
    restored?;
    result
}
pub(crate) fn start_if_requested(app: &AppHandle) {
    let Ok(mode) = env::var("STUDIO_RUN_QA") else {
        return;
    };
    if env::var_os("STUDIO_QA_ROOT").is_none()
        || !matches!(
            mode.as_str(),
            "full" | "live" | "capture" | "lifecycle" | "crash" | "recover"
        )
    {
        return;
    }
    // The harness has no interactive UI; keep it from occluding the real renderer
    // whose painting/capture behaviour is under test. Release builds omit this module.
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = tauri::async_runtime::block_on(exercise(&app, &mode));
        let _ = record(
            "finished",
            json!({"success":result.is_ok(),"error":result.as_ref().err().map(|e|e.code.clone()),"reason":result.as_ref().err().map(|e|e.message.clone())}),
        );
        app.exit(if result.is_ok() { 0 } else { 1 });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_real_engine_visual_fixtures() {
        let Ok(destination) = env::var("STUDIO_QA_FIXTURES") else {
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let store = theme_library::ThemeStore {
            root: temp.path().into(),
        };
        let mut fixtures = Vec::new();
        for (name, color) in [
            ("light", [235, 239, 241]),
            ("dark", [8, 16, 25]),
            ("saturated", [231, 25, 98]),
        ] {
            let mut view = store
                .import(&format!("{name}.png"), &fixture_bytes(color))
                .unwrap();
            let hero = fs::read(
                store
                    .asset_file(&view.document.asset_id, "hero.jpg")
                    .unwrap(),
            )
            .unwrap();
            view.image_path = format!("data:image/jpeg;base64,{}", base64_encode(&hero));
            fixtures.push(json!({"name":name,"view":view}));
        }
        for (name, relative) in [
            ("portrait", "themes/pink-lijiajia/theme.codedrobe-theme"),
            (
                "landscape",
                "themes/oriental-landscape/theme.codedrobe-theme",
            ),
        ] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join(relative);
            if path.exists() {
                let package: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
                let asset = &package["assets"]["images"]["hero"];
                let filename = asset["filename"].as_str().unwrap();
                let data = base64_decode(asset["base64"].as_str().unwrap()).unwrap();
                let mut view = store.import(filename, &data).unwrap();
                view.image_path = format!(
                    "data:image/jpeg;base64,{}",
                    base64_encode(
                        &fs::read(
                            store
                                .asset_file(&view.document.asset_id, "hero.jpg")
                                .unwrap()
                        )
                        .unwrap()
                    )
                );
                fixtures.push(json!({"name":name,"view":view}));
            }
        }
        let sources = fixtures;
        let mut fixtures = Vec::new();
        for source in sources {
            let document: theme_library::ThemeDocument =
                serde_json::from_value(source["view"]["document"].clone()).unwrap();
            for style in [
                theme_compiler::StyleId::AiryLight,
                theme_compiler::StyleId::WarmPaper,
                theme_compiler::StyleId::CalmDark,
            ] {
                let mut view = store
                    .update(
                        &document.draft_id,
                        theme_library::DraftUpdate {
                            name: document.name.clone(),
                            controls: theme_compiler::controls_for(
                                &theme_compiler::StyleSelection {
                                    id: style,
                                    version: 2,
                                },
                            ),
                            style: Some(theme_compiler::StyleSelection {
                                id: style,
                                version: 2,
                            }),
                            reset_style: true,
                            sequence: store.read_draft(&document.draft_id).unwrap().edit_sequence
                                + 1,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                view.image_path = source["view"]["imagePath"].as_str().unwrap().into();
                let style_id = serde_json::to_value(style).unwrap();
                fixtures.push(json!({"name":format!("{}-{}",source["name"].as_str().unwrap(),style_id.as_str().unwrap()),"view":view}));
            }
        }
        let path = PathBuf::from(destination);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        atomic_write(&path, &serde_json::to_vec(&fixtures).unwrap()).unwrap();
        assert_eq!(fixtures.len(), 15);
    }
}
