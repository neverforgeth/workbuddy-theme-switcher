//! Drafts and immutable revisions. All paths are derived here, never supplied by the UI.
use super::*;
use theme_engine::{CompiledTheme, ImportedImage};
use theme_model::StyleAdvice;

// A reader must not mistake an in-process pre-commit journal for a crashed save.
// No WorkBuddy or application-state locks are acquired while this guard is held.
static PUBLICATION_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThemeDocument {
    pub schema_version: u32,
    pub draft_id: String,
    pub theme_id: Option<String>,
    pub revision: Option<u32>,
    pub name: String,
    pub asset_id: String,
    pub source_filename: String,
    pub width: u32,
    pub height: u32,
    pub background: String,
    pub extracted_accent: String,
    pub controls: ThemeControls,
    pub advice: StyleAdvice,
    pub compiled: CompiledTheme,
    pub edit_sequence: u64,
    pub saved_sequence: Option<u64>,
    #[serde(default)]
    pub design: Option<region_theme::DesignAdvice>,
    #[serde(default)]
    pub background_samples: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<theme_compiler::StyleSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<theme_compiler::ImageAnalysis>,
    #[serde(default)]
    pub manual_overrides: theme_compiler::ManualOverrides,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DraftView {
    pub document: ThemeDocument,
    pub image_path: String,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DraftUpdate {
    pub name: String,
    pub controls: ThemeControls,
    pub sequence: u64,
    #[serde(default)]
    pub style: Option<theme_compiler::StyleSelection>,
    #[serde(default)]
    pub reset_style: bool,
    #[serde(default)]
    pub design: Option<region_theme::DesignAdvice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThemeRef {
    pub id: String,
    pub revision: Option<u32>,
}
impl ThemeRef {
    pub fn key(&self) -> String {
        match self.revision {
            Some(revision) => format!("{}@{revision}", self.id),
            None => self.id.clone(),
        }
    }
    pub fn parse(key: &str) -> AppResult<Self> {
        if let Some((id, revision)) = key.split_once('@') {
            check_theme_id(id)?;
            let revision = revision
                .parse::<u32>()
                .ok()
                .filter(|v| *v > 0)
                .ok_or_else(|| error("主题修订号无效。"))?;
            Ok(Self {
                id: id.into(),
                revision: Some(revision),
            })
        } else {
            Ok(Self {
                id: key.into(),
                revision: None,
            })
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LibraryItem {
    pub reference: ThemeRef,
    pub name: String,
    pub description: String,
    pub preview_path: String,
    pub editable: bool,
    pub is_custom: bool,
    pub palette: Option<theme_engine::Palette>,
    pub compatibility: String,
}

pub(crate) struct ThemeStore {
    pub root: PathBuf,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingSave {
    before: ThemeDocument,
    reference: ThemeRef,
    previous_head: Option<u32>,
}
fn error(message: &str) -> AppError {
    AppError::new("THEME_STORE_ERROR", message)
}
fn io_error(_: io::Error) -> AppError {
    error("无法读写主题文件；原有主题未被覆盖。")
}
fn check_id(value: &str) -> AppResult<()> {
    if value.len() != 32 || !value.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(error("主题资源标识无效。"));
    }
    Ok(())
}
fn check_theme_id(value: &str) -> AppResult<()> {
    check_id(
        value
            .strip_prefix("custom-v2-")
            .ok_or_else(|| error("主题标识无效。"))?,
    )
}
fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
fn validate_name(value: &str) -> AppResult<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 48 || value.chars().any(char::is_control) {
        return Err(error("主题名称应为 1–48 个字符。"));
    }
    Ok(value.into())
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> AppResult<T> {
    let metadata = fs::metadata(path).map_err(io_error)?;
    if metadata.len() > 2 * 1024 * 1024 {
        return Err(error("主题文档超出大小限制。"));
    }
    serde_json::from_slice(&fs::read(path).map_err(io_error)?).map_err(|_| error("主题文档损坏。"))
}
fn write_json(path: &Path, value: &impl Serialize) -> AppResult<()> {
    fs::create_dir_all(path.parent().ok_or_else(|| error("路径无效。"))?).map_err(io_error)?;
    atomic_write(
        path,
        &serde_json::to_vec_pretty(value).map_err(|_| error("无法序列化主题。"))?,
    )
    .map_err(io_error)
}

impl ThemeStore {
    pub fn production() -> AppResult<Self> {
        Ok(Self {
            root: runtime_root()?.join("studio"),
        })
    }
    fn draft_path(&self, id: &str) -> AppResult<PathBuf> {
        check_id(id)?;
        Ok(self.root.join("drafts").join(format!("{id}.json")))
    }
    fn asset_dir(&self, id: &str) -> AppResult<PathBuf> {
        check_id(id)?;
        Ok(self.root.join("assets").join(id))
    }
    fn theme_dir(&self, id: &str) -> AppResult<PathBuf> {
        check_theme_id(id)?;
        Ok(self.root.join("library").join(id))
    }
    pub fn revision_dir(&self, reference: &ThemeRef) -> AppResult<PathBuf> {
        let revision = reference
            .revision
            .filter(|v| *v > 0)
            .ok_or_else(|| error("缺少主题修订号。"))?;
        Ok(self.theme_dir(&reference.id)?.join(format!("r{revision}")))
    }
    pub fn import(&self, filename: &str, bytes: &[u8]) -> AppResult<DraftView> {
        let image = theme_engine::import_image(filename, bytes)?;
        self.import_processed(filename, bytes, image)
    }
    pub(crate) fn import_processed(
        &self,
        filename: &str,
        bytes: &[u8],
        image: ImportedImage,
    ) -> AppResult<DraftView> {
        let asset_id = new_id();
        let assets = self.asset_dir(&asset_id)?;
        fs::create_dir_all(&assets).map_err(io_error)?;
        let ext = source_image_extension(Path::new(filename), bytes)?;
        atomic_write(&assets.join(format!("source.{ext}")), bytes).map_err(io_error)?;
        atomic_write(&assets.join("hero.jpg"), &image.hero).map_err(io_error)?;
        atomic_write(&assets.join("thumbnail.jpg"), &image.thumbnail).map_err(io_error)?;
        let style = theme_compiler::StyleSelection {
            id: image.analysis.recommended,
            version: 2,
        };
        let controls = theme_compiler::controls_for(&style);
        let advice = StyleAdvice::default();
        let overrides = theme_compiler::ManualOverrides::default();
        let (compiled, design) =
            theme_compiler::compile(&style, &image.analysis, &controls, &overrides, None)?;
        let background_samples = image.analysis.samples.clone();
        let document = ThemeDocument {
            schema_version: 3,
            draft_id: new_id(),
            theme_id: None,
            revision: None,
            name: "我的图片主题".into(),
            asset_id,
            source_filename: Path::new(filename)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            width: image.width,
            height: image.height,
            background: image.background,
            extracted_accent: image.accent,
            controls,
            advice,
            compiled,
            edit_sequence: 0,
            saved_sequence: None,
            design: Some(design),
            background_samples,
            style: Some(style),
            analysis: Some(image.analysis),
            manual_overrides: overrides,
        };
        self.write_draft(&document)?;
        self.view(document, None)
    }
    /// Explicit opt-in: neither reading, compiling nor publishing the copy writes the source.
    pub(crate) fn copy_fusion(
        &self,
        id: &str,
        checkpoint: impl Fn() -> AppResult<()>,
    ) -> AppResult<DraftView> {
        let _publication = PUBLICATION_LOCK
            .lock()
            .map_err(|_| error("保存状态暂不可用。"))?;
        if self.pending_save_path(id)?.exists() {
            return Err(error("原稿有待恢复的保存操作，请先重新打开原稿。"));
        }
        let original: ThemeDocument = read_json(&self.draft_path(id)?)?;
        self.validate(&original)?;
        if original.draft_id != id {
            return Err(error("草稿标识不一致。"));
        }
        checkpoint()?;
        let analysis = if original.analysis.as_ref().is_some_and(|a| a.version == 2) {
            original.analysis.clone().unwrap()
        } else {
            let path = self.asset_file(&original.asset_id, "hero.jpg")?;
            if fs::metadata(&path).map_err(io_error)?.len() > theme_engine::MAX_IMAGE_BYTES as u64 {
                return Err(error("受控图片超出大小限制。"));
            }
            theme_engine::import_image("hero.jpg", &fs::read(path).map_err(io_error)?)?.analysis
        };
        checkpoint()?;
        let style = theme_compiler::StyleSelection {
            id: original
                .style
                .as_ref()
                .map_or(analysis.recommended, |s| s.id),
            version: 2,
        };
        let mut controls = theme_compiler::controls_for(&style);
        controls.accent = original.controls.accent.clone();
        let mut position = original.design.clone();
        if let Some(d) = &mut position {
            d.veil = 0;
        }
        let overrides = theme_compiler::ManualOverrides {
            global: controls.accent.is_some(),
            position: position
                .as_ref()
                .is_some_and(|p| p.background_x != 50 || p.background_y != 50),
            ..Default::default()
        };
        let (compiled, design) =
            theme_compiler::compile(&style, &analysis, &controls, &overrides, position.as_ref())?;
        let doc = ThemeDocument {
            draft_id: new_id(),
            schema_version: 3,
            theme_id: None,
            revision: None,
            edit_sequence: 0,
            saved_sequence: None,
            controls,
            compiled,
            design: Some(design),
            style: Some(style),
            background_samples: analysis.samples.clone(),
            analysis: Some(analysis),
            manual_overrides: overrides,
            ..original
        };
        self.validate(&doc)?;
        let view = self.view(
            doc,
            Some("已创建新版融合副本，原稿、旧修订与当前 WorkBuddy 效果均未改变。".into()),
        )?;
        checkpoint()?;
        let path = self.draft_path(&view.document.draft_id)?;
        write_json(&path, &view.document)?;
        if let Err(e) = write_json(&self.root.join("last-draft.json"), &view.document.draft_id) {
            // Only the just-created unpublished copy is removed; the original is never a target.
            let _ = fs::remove_file(path);
            return Err(e);
        }
        Ok(view)
    }
    fn write_draft(&self, doc: &ThemeDocument) -> AppResult<()> {
        write_json(&self.draft_path(&doc.draft_id)?, doc)?;
        write_json(&self.root.join("last-draft.json"), &doc.draft_id)
    }
    pub fn read_draft(&self, id: &str) -> AppResult<ThemeDocument> {
        let _publication = PUBLICATION_LOCK
            .lock()
            .map_err(|_| error("保存状态暂不可用。"))?;
        self.recover_save(id)?;
        let mut doc: ThemeDocument = read_json(&self.draft_path(id)?)?;
        self.validate(&doc)?;
        if doc.draft_id != id {
            return Err(error("草稿标识不一致。"));
        }
        if theme_compiler::repair_draft_background(&mut doc)? {
            // Draft-only atomic upgrade; the sequence marks this as an unsaved compatibility edit.
            write_json(&self.draft_path(id)?, &doc)?;
        }
        Ok(doc)
    }
    fn pending_save_path(&self, id: &str) -> AppResult<PathBuf> {
        check_id(id)?;
        Ok(self
            .root
            .join("save-transactions")
            .join(format!("{id}.json")))
    }
    fn head(&self, theme_id: &str) -> AppResult<Option<u32>> {
        let path = self.theme_dir(theme_id)?.join("head.json");
        if path.exists() {
            read_json(&path).map(Some)
        } else {
            Ok(None)
        }
    }
    fn recover_save(&self, id: &str) -> AppResult<()> {
        let path = self.pending_save_path(id)?;
        if !path.exists() {
            return Ok(());
        }
        let pending: PendingSave = read_json(&path)?;
        if pending.before.draft_id != id
            || pending
                .before
                .theme_id
                .as_ref()
                .is_some_and(|theme| theme != &pending.reference.id)
        {
            return Err(error("保存恢复记录与草稿不匹配，未改动文件。"));
        }
        self.validate(&pending.before)?;
        let after = self.read_revision(&pending.reference)?;
        if after.draft_id != id {
            return Err(error("保存恢复修订与草稿不匹配。"));
        }
        let head = self.head(&pending.reference.id)?;
        let desired = if head == pending.reference.revision {
            &after
        } else if head == pending.previous_head {
            &pending.before
        } else {
            return Err(error("保存恢复期间主题已变化，请保留恢复记录后检查。"));
        };
        let current: ThemeDocument = read_json(&self.draft_path(id)?)?;
        let value = serde_json::to_value(&current).map_err(|_| error("无法检查保存恢复记录。"))?;
        if value != serde_json::to_value(&pending.before).unwrap()
            && value != serde_json::to_value(&after).unwrap()
        {
            return Err(error("草稿在保存恢复期间已被修改，未覆盖新内容。"));
        }
        if value != serde_json::to_value(desired).unwrap() {
            write_json(&self.draft_path(id)?, desired)?;
        }
        fs::remove_file(path).map_err(io_error)
    }
    fn published_revision(&self, reference: &ThemeRef) -> AppResult<ThemeDocument> {
        let revision = reference
            .revision
            .ok_or_else(|| error("缺少主题修订号。"))?;
        let head = self
            .head(&reference.id)?
            .ok_or_else(|| error("主题尚未发布。"))?;
        if revision > head
            || (revision != head
                && self
                    .revision_dir(reference)?
                    .join("pending-publication.json")
                    .exists())
        {
            return Err(error("草稿引用的修订尚未发布，请重新保存。"));
        }
        self.read_revision(reference)
    }
    pub fn latest(&self) -> AppResult<Option<DraftView>> {
        let path = self.root.join("last-draft.json");
        if !path.exists() {
            return Ok(None);
        }
        let id: String = read_json(&path)?;
        self.view(self.read_draft(&id)?, None).map(Some)
    }
    fn validate(&self, doc: &ThemeDocument) -> AppResult<()> {
        if !matches!(doc.schema_version, 1 | 2 | 3) {
            return Err(error("此主题文档版本暂不支持编辑。"));
        }
        if doc.schema_version == 3 {
            theme_compiler::validate(
                doc.style.as_ref().ok_or_else(|| error("缺少风格版本。"))?,
                doc.analysis
                    .as_ref()
                    .ok_or_else(|| error("缺少分析缓存。"))?,
            )?;
        }
        check_id(&doc.draft_id)?;
        check_id(&doc.asset_id)?;
        validate_name(&doc.name)?;
        validate_controls(&doc.controls)?;
        if doc.schema_version >= 2 && doc.design.is_none() {
            return Err(error("分区主题缺少设计参数。"));
        }
        if let Some(design) = &doc.design {
            region_theme::validate(design)?;
        }
        if doc.background_samples.len() > 256
            || doc
                .background_samples
                .iter()
                .any(|v| parse_hex_color(v).is_none())
        {
            return Err(error("图片分析数据无效。"));
        }
        if let Some(id) = &doc.theme_id {
            check_theme_id(id)?;
        }
        if parse_hex_color(&doc.background).is_none()
            || parse_hex_color(&doc.extracted_accent).is_none()
        {
            return Err(error("主题颜色无效。"));
        }
        // Compiled documents are local artifacts, not an arbitrary CSS import interface.
        if doc.compiled.css.contains("url(")
            || doc.compiled.css.contains("@import")
            || doc.compiled.css.contains("</")
        {
            return Err(error("主题文档包含不安全样式。"));
        }
        self.asset_file(&doc.asset_id, "hero.jpg")?;
        Ok(())
    }
    pub fn asset_file(&self, id: &str, name: &str) -> AppResult<PathBuf> {
        if !matches!(
            name,
            "hero.jpg" | "thumbnail.jpg" | "source.jpg" | "source.png" | "source.webp"
        ) {
            return Err(error("不允许的资源文件。"));
        }
        let path = self.asset_dir(id)?.join(name);
        let canonical = path.canonicalize().map_err(io_error)?;
        let root = self.root.canonicalize().map_err(io_error)?;
        if !canonical.starts_with(root) {
            return Err(error("资源不在应用数据目录内。"));
        }
        Ok(path)
    }
    pub fn view(&self, document: ThemeDocument, warning: Option<String>) -> AppResult<DraftView> {
        let image_path = self
            .asset_file(&document.asset_id, "hero.jpg")?
            .to_string_lossy()
            .into();
        Ok(DraftView {
            document,
            image_path,
            warning,
        })
    }
    pub fn update(&self, id: &str, update: DraftUpdate) -> AppResult<DraftView> {
        let mut doc = self.read_draft(id)?;
        if update.sequence <= doc.edit_sequence {
            return self.view(doc, None);
        }
        let name = validate_name(&update.name)?;
        // v1 conversion without cached samples processes the controlled derivative once, never the source.
        if (update.style.is_some() || update.reset_style)
            && doc.analysis.is_none()
            && doc.background_samples.is_empty()
        {
            doc.background_samples = region_theme::sample_colors(
                &fs::read(self.asset_file(&doc.asset_id, "hero.jpg")?).map_err(io_error)?,
            )?;
        }
        let doc = theme_compiler::edit(doc, DraftUpdate { name, ..update })?;
        // Editing an existing draft must not republish the global latest-draft index.
        // One atomic file replacement is the edit commit, so index failures cannot
        // report a failed edit after advancing its sequence.
        write_json(&self.draft_path(&doc.draft_id)?, &doc)?;
        self.view(doc, None)
    }
    pub fn update_design(
        &self,
        id: &str,
        sequence: u64,
        design: Option<region_theme::DesignAdvice>,
    ) -> AppResult<DraftView> {
        let doc = self.read_draft(id)?;
        if doc.edit_sequence != sequence {
            return Err(error("草稿已变化，未覆盖新参数；请重试。"));
        }
        let design = design.or_else(|| Some(region_theme::defaults(&doc.compiled.palette)));
        self.update(
            id,
            DraftUpdate {
                name: doc.name,
                controls: doc.controls,
                sequence: sequence + 1,
                design,
                ..Default::default()
            },
        )
    }
    pub fn save(&self, id: &str) -> AppResult<ThemeRef> {
        let doc = self.read_draft(id)?;
        if doc.saved_sequence == Some(doc.edit_sequence) {
            if let (Some(id), Some(revision)) = (doc.theme_id.clone(), doc.revision) {
                let reference = ThemeRef {
                    id,
                    revision: Some(revision),
                };
                let mut published = self.published_revision(&reference)?;
                published.draft_id = doc.draft_id.clone();
                if serde_json::to_value(&published).unwrap() != serde_json::to_value(&doc).unwrap()
                {
                    return Err(error("草稿与其已保存修订不一致，未返回未确认的修订。"));
                }
                return Ok(reference);
            }
        }
        self.publish_document(doc.clone(), Some(doc))
    }
    fn publish_document(
        &self,
        mut doc: ThemeDocument,
        before: Option<ThemeDocument>,
    ) -> AppResult<ThemeRef> {
        let _publication = PUBLICATION_LOCK
            .lock()
            .map_err(|_| error("保存状态暂不可用。"))?;
        let theme_id = doc
            .theme_id
            .clone()
            .unwrap_or_else(|| format!("custom-v2-{}", new_id()));
        let dir = self.theme_dir(&theme_id)?;
        fs::create_dir_all(&dir).map_err(io_error)?;
        let previous_head = self.head(&theme_id)?;
        let last = previous_head.unwrap_or(0);
        // The current head itself proves this older revision committed, even if cleanup was interrupted.
        if let Some(previous) = previous_head {
            let marker = dir
                .join(format!("r{previous}"))
                .join("pending-publication.json");
            if marker.exists() {
                fs::remove_file(marker).map_err(io_error)?;
            }
        }
        let mut revision = last
            .checked_add(1)
            .ok_or_else(|| error("主题修订号溢出。"))?;
        // A crash between directory publication and head publication must not block future saves.
        while dir.join(format!("r{revision}")).exists() {
            revision = revision
                .checked_add(1)
                .ok_or_else(|| error("主题修订号溢出。"))?;
        }
        let reference = ThemeRef {
            id: theme_id.clone(),
            revision: Some(revision),
        };
        let staging = dir.join(format!("pending-{}", new_id()));
        fs::create_dir(&staging).map_err(io_error)?;
        doc.theme_id = Some(theme_id);
        doc.revision = Some(revision);
        doc.saved_sequence = Some(doc.edit_sequence);
        let hero = fs::read(self.asset_file(&doc.asset_id, "hero.jpg")?).map_err(io_error)?;
        write_json(&staging.join("document.json"), &doc)?;
        write_json(
            &staging.join("theme.codedrobe-theme"),
            &theme_engine::package(&runtime_id(&reference), &doc.name, &doc.compiled, &hero),
        )?;
        write_json(&staging.join("pending-publication.json"), &reference)?;
        // Readers see a revision only after the complete directory and then head are published.
        let target = self.revision_dir(&reference)?;
        if target.exists() {
            return Err(error("修订号冲突，未覆盖既有主题。"));
        }
        fs::rename(&staging, &target).map_err(io_error)?;
        let transaction = if let Some(before) = before {
            let path = self.pending_save_path(&doc.draft_id)?;
            write_json(
                &path,
                &PendingSave {
                    before,
                    reference: reference.clone(),
                    previous_head,
                },
            )?;
            if let Err(failure) = write_json(&self.draft_path(&doc.draft_id)?, &doc) {
                self.recover_save(&doc.draft_id)?;
                return Err(failure);
            }
            Some(path)
        } else {
            None
        };
        // Sole publication/commit point. Nothing fallible after this may report a failed save.
        if let Err(failure) = write_json(&dir.join("head.json"), &revision) {
            if self.head(&reference.id)? != Some(revision) {
                if transaction.is_some() {
                    self.recover_save(&doc.draft_id)?;
                }
                return Err(failure);
            }
        }
        let _ = fs::remove_file(target.join("pending-publication.json"));
        if let Some(path) = transaction {
            let _ = fs::remove_file(path);
        }
        Ok(reference)
    }
    pub fn read_revision(&self, reference: &ThemeRef) -> AppResult<ThemeDocument> {
        let doc: ThemeDocument = read_json(&self.revision_dir(reference)?.join("document.json"))?;
        self.validate(&doc)?;
        if doc.theme_id.as_deref() != Some(&reference.id) || doc.revision != reference.revision {
            return Err(error("修订元数据不一致。"));
        }
        Ok(doc)
    }
    pub fn open(&self, reference: &ThemeRef) -> AppResult<DraftView> {
        let mut doc = self.read_revision(reference)?;
        doc.draft_id = new_id();
        self.write_draft(&doc)?;
        self.view(self.read_draft(&doc.draft_id)?, None)
    }
    pub fn rename(&self, reference: &ThemeRef, name: &str) -> AppResult<ThemeRef> {
        let name = validate_name(name)?;
        let mut doc = self.published_revision(reference)?;
        doc.name = name;
        doc.edit_sequence = doc
            .edit_sequence
            .checked_add(1)
            .ok_or_else(|| error("草稿序号溢出。"))?;
        self.publish_document(doc, None)
    }
    pub fn list(&self) -> AppResult<Vec<LibraryItem>> {
        let mut items = Vec::new();
        let root = self.root.join("library");
        if !root.exists() {
            return Ok(items);
        }
        for entry in fs::read_dir(root).map_err(io_error)?.flatten() {
            let id = entry.file_name().to_string_lossy().into_owned();
            if check_theme_id(&id).is_err() {
                continue;
            }
            let Ok(revision) = read_json::<u32>(&entry.path().join("head.json")) else {
                continue;
            };
            let reference = ThemeRef {
                id,
                revision: Some(revision),
            };
            if let Ok(doc) = self.read_revision(&reference) {
                items.push(LibraryItem {
                    reference,
                    name: doc.name,
                    description: "图片主题 · 可继续编辑".into(),
                    preview_path: self
                        .asset_file(&doc.asset_id, "hero.jpg")?
                        .to_string_lossy()
                        .into(),
                    editable: true,
                    is_custom: true,
                    palette: Some(doc.compiled.palette),
                    compatibility: "5.2.6 样板；应用时另做运行验证".into(),
                });
            }
        }
        items.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(items)
    }
    pub fn record(&self, reference: &ThemeRef) -> AppResult<ThemeRecord> {
        let doc = self.read_revision(reference)?;
        Ok(ThemeRecord {
            id: reference.key(),
            name: doc.name,
            description: "图片主题 · 可继续编辑".into(),
            package_path: self
                .revision_dir(reference)?
                .join("theme.codedrobe-theme")
                .to_string_lossy()
                .into(),
            preview_path: self
                .asset_file(&doc.asset_id, "hero.jpg")?
                .to_string_lossy()
                .into(),
            verified_work_buddy_version: "5.2.6 样板；运行时验证".into(),
            theme_version: format!("1.4.0-r{}", reference.revision.unwrap()),
            runtime_theme_id: runtime_id(reference),
            is_custom: true,
        })
    }
    pub fn trash(&self, id: &str) -> AppResult<()> {
        let source = self.theme_dir(id)?;
        if !source.exists() {
            return Err(error("主题不存在。"));
        }
        let source = source.canonicalize().map_err(io_error)?;
        let library = self.root.join("library").canonicalize().map_err(io_error)?;
        if source.parent() != Some(library.as_path()) {
            return Err(error("删除目标不在主题库。"));
        }
        let trash = self.root.join("trash");
        fs::create_dir_all(&trash).map_err(io_error)?;
        fs::rename(source, trash.join(format!("{id}-{}", new_id()))).map_err(io_error)
    }
}
pub(crate) fn runtime_id(reference: &ThemeRef) -> String {
    format!(
        "workbuddy-{}-r{}",
        reference.id,
        reference.revision.unwrap_or(0)
    )
}
pub(crate) fn reference_from_runtime(id: &str) -> Option<ThemeRef> {
    let id = id.strip_prefix("workbuddy-")?;
    let (theme, revision) = id.rsplit_once("-r")?;
    if check_theme_id(theme).is_err() {
        return None;
    }
    Some(ThemeRef {
        id: theme.into(),
        revision: Some(revision.parse().ok()?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn switching_v2_style_preserves_the_latest_slider_intent_in_the_same_submission() {
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let d = store.import("test.png", &fixture()).unwrap().document;
        let style = theme_compiler::StyleSelection {
            id: theme_compiler::StyleId::CalmDark,
            version: 2,
        };
        let controls = ThemeControls {
            brightness: 17,
            blur: 0,
            ..theme_compiler::controls_for(&style)
        };
        let changed = store
            .update(
                &d.draft_id,
                DraftUpdate {
                    name: d.name.clone(),
                    controls: controls.clone(),
                    style: Some(style.clone()),
                    reset_style: true,
                    sequence: 1,
                    ..Default::default()
                },
            )
            .unwrap()
            .document;
        assert_eq!(changed.controls.brightness, 17);
        assert_eq!(changed.controls.blur, 0);
        assert_eq!(changed.style, Some(style.clone()));
        assert_eq!(store.list().unwrap().len(), 0);
        let reset = store
            .update(
                &d.draft_id,
                DraftUpdate {
                    name: d.name,
                    controls: theme_compiler::controls_for(&style),
                    style: Some(style),
                    reset_style: true,
                    sequence: 2,
                    ..Default::default()
                },
            )
            .unwrap()
            .document;
        assert_eq!(reset.controls.brightness, -12);
        assert_eq!(reset.controls.blur, 0);
        assert!(!reset.manual_overrides.global);
    }
    #[test]
    #[cfg(windows)]
    fn failed_or_cancelled_fusion_copy_leaves_source_and_latest_unchanged() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let d = store.import("test.png", &fixture()).unwrap().document;
        let draft_path = store.draft_path(&d.draft_id).unwrap();
        let before = fs::read(&draft_path).unwrap();
        let latest_path = store.root.join("last-draft.json");
        let latest = fs::read(&latest_path).unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&latest_path)
            .unwrap();
        assert!(store.copy_fusion(&d.draft_id, || Ok(())).is_err());
        drop(lock);
        assert_eq!(fs::read(&draft_path).unwrap(), before);
        assert_eq!(fs::read(&latest_path).unwrap(), latest);
        assert_eq!(fs::read_dir(store.root.join("drafts")).unwrap().count(), 1);
        assert!(store
            .copy_fusion(&d.draft_id, || Err(error("cancelled")))
            .is_err());
        assert_eq!(fs::read(&draft_path).unwrap(), before);
        assert_eq!(fs::read(&latest_path).unwrap(), latest);
    }
    #[test]
    fn fusion_copy_preserves_original_bytes_and_starts_an_independent_zero_blur_draft() {
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let mut old = store.import("test.png", &fixture()).unwrap().document;
        old.style = Some(theme_compiler::StyleSelection {
            id: theme_compiler::StyleId::WarmPaper,
            version: 1,
        });
        old.analysis = Some(theme_compiler::analyze(old.background_samples.clone()).unwrap());
        old.controls = theme_compiler::controls(theme_compiler::StyleId::WarmPaper);
        old.controls.accent = Some("#326B58".into());
        old.design.as_mut().unwrap().background_x = 19;
        old.design.as_mut().unwrap().background_y = 77;
        (old.compiled, old.design) = {
            let (css, d) = theme_compiler::compile(
                old.style.as_ref().unwrap(),
                old.analysis.as_ref().unwrap(),
                &old.controls,
                &old.manual_overrides,
                old.design.as_ref(),
            )
            .unwrap();
            (css, Some(d))
        };
        store.write_draft(&old).unwrap();
        let reference = store.save(&old.draft_id).unwrap();
        let original = fs::read(store.draft_path(&old.draft_id).unwrap()).unwrap();
        let package = store
            .revision_dir(&reference)
            .unwrap()
            .join("theme.codedrobe-theme");
        let package_bytes = fs::read(&package).unwrap();
        let copied = store
            .copy_fusion(&old.draft_id, || Ok(()))
            .unwrap()
            .document;
        assert_ne!(copied.draft_id, old.draft_id);
        assert!(copied.theme_id.is_none());
        assert_eq!(copied.name, old.name);
        assert_eq!(copied.asset_id, old.asset_id);
        assert_eq!(copied.style.as_ref().unwrap().version, 2);
        assert_eq!(copied.analysis.as_ref().unwrap().version, 2);
        assert_eq!(copied.controls.blur, 0);
        assert_eq!(copied.controls.accent, old.controls.accent);
        assert_eq!(copied.design.as_ref().unwrap().background_x, 19);
        assert_eq!(copied.design.as_ref().unwrap().background_y, 77);
        assert!(copied.manual_overrides.regions.is_empty());
        assert_eq!(store.list().unwrap().len(), 1);
        assert_eq!(
            fs::read(store.draft_path(&old.draft_id).unwrap()).unwrap(),
            original
        );
        assert_eq!(fs::read(&package).unwrap(), package_bytes);
        assert_eq!(
            store.latest().unwrap().unwrap().document.draft_id,
            copied.draft_id
        );
        let copy_ref = store.save(&copied.draft_id).unwrap();
        assert_ne!(copy_ref.id, reference.id);
        assert_eq!(store.open(&copy_ref).unwrap().document.controls.blur, 0);
    }
    #[test]
    fn old_offline_draft_gets_one_background_repair_without_rewriting_saved_revision() {
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let mut old = store.import("test.png", &fixture()).unwrap().document;
        old.style = Some(theme_compiler::StyleSelection {
            id: theme_compiler::StyleId::AiryLight,
            version: 1,
        });
        old.analysis = Some(theme_compiler::analyze(old.background_samples.clone()).unwrap());
        let (compiled, design) = theme_compiler::compile(
            old.style.as_ref().unwrap(),
            old.analysis.as_ref().unwrap(),
            &old.controls,
            &old.manual_overrides,
            None,
        )
        .unwrap();
        old.compiled = compiled;
        old.design = Some(design);
        old.compiled.template_version = "workbuddy-5.2.6-offline-v3.1".into();
        old.compiled.css = old
            .compiled
            .css
            .replace(include_str!("workbuddy-background-compat.css"), "")
            .replace(
                ":where(#workbuddy-menubar-container)",
                "#workbuddy-menubar-container",
            );
        let reference = store.publish_document(old, None).unwrap();
        let saved = store.read_revision(&reference).unwrap();
        let package_path = store
            .revision_dir(&reference)
            .unwrap()
            .join("theme.codedrobe-theme");
        let package_before = fs::read(&package_path).unwrap();
        store.write_draft(&saved).unwrap();
        let repaired = store.latest().unwrap().unwrap().document;
        assert_eq!(
            repaired.compiled.template_version,
            "workbuddy-5.2.6-offline-v3.2"
        );
        assert_eq!(repaired.edit_sequence, saved.edit_sequence + 1);
        assert_eq!(repaired.saved_sequence, None);
        assert_eq!(repaired.asset_id, saved.asset_id);
        assert_eq!(
            serde_json::to_value(&repaired.controls).unwrap(),
            serde_json::to_value(&saved.controls).unwrap()
        );
        assert_eq!(
            store.read_draft(&repaired.draft_id).unwrap().edit_sequence,
            repaired.edit_sequence
        );
        assert_eq!(
            store.read_revision(&reference).unwrap().compiled.css,
            saved.compiled.css
        );
        assert_eq!(fs::read(package_path).unwrap(), package_before);
        let reopened = store.open(&reference).unwrap().document;
        assert_eq!(
            reopened.compiled.template_version,
            "workbuddy-5.2.6-offline-v3.2"
        );
        assert_eq!(store.save(&repaired.draft_id).unwrap().revision, Some(2));
    }
    #[test]
    fn atomic_edits_preserve_overrides_reset_deliberately_and_never_decode_on_update() {
        use theme_compiler::{StyleId, StyleSelection};
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let mut d = store.import("test.png", &fixture()).unwrap().document;
        let recommended = d.analysis.as_ref().unwrap().recommended;
        let mut design = d.design.clone().unwrap();
        design.background_x = 23;
        design.background_y = 71;
        design
            .regions
            .get_mut(&region_theme::Region::Composer)
            .unwrap()
            .background = "#334455".into();
        d = store
            .update(
                &d.draft_id,
                DraftUpdate {
                    name: "保留名称".into(),
                    controls: ThemeControls {
                        brightness: 25,
                        ..d.controls.clone()
                    },
                    design: Some(design),
                    sequence: 1,
                    ..Default::default()
                },
            )
            .unwrap()
            .document;
        assert_eq!(d.edit_sequence, 1);
        assert_eq!(d.controls.brightness, 25);
        assert_eq!(d.manual_overrides.regions.len(), 1);
        let analysis = serde_json::to_value(&d.analysis).unwrap();
        let original = store.save(&d.draft_id).unwrap();
        let css = d.compiled.css.clone();
        // Corrupt the derivative in this disposable store: updates must not decode it.
        fs::write(
            store.asset_file(&d.asset_id, "hero.jpg").unwrap(),
            b"not an image",
        )
        .unwrap();
        let mut timings = Vec::new();
        for sequence in 2..=61 {
            let started = Instant::now();
            d = store
                .update(
                    &d.draft_id,
                    DraftUpdate {
                        name: d.name.clone(),
                        controls: ThemeControls {
                            brightness: (sequence % 30) as i16,
                            ..d.controls.clone()
                        },
                        sequence,
                        ..Default::default()
                    },
                )
                .unwrap()
                .document;
            timings.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        timings.sort_by(f64::total_cmp);
        println!(
            "v3 cached compile + atomic draft persistence P95 {:.2}ms; no IPC/CDP/capture included",
            timings[56]
        );
        assert!(timings[56] < 200.0);
        assert_eq!(serde_json::to_value(&d.analysis).unwrap(), analysis);
        assert_eq!(d.analysis.as_ref().unwrap().recommended, recommended);
        let switched = store
            .update(
                &d.draft_id,
                DraftUpdate {
                    name: d.name.clone(),
                    controls: theme_compiler::controls_for(&StyleSelection {
                        id: StyleId::WarmPaper,
                        version: 2,
                    }),
                    style: Some(StyleSelection {
                        id: StyleId::WarmPaper,
                        version: 2,
                    }),
                    reset_style: true,
                    sequence: 62,
                    ..Default::default()
                },
            )
            .unwrap()
            .document;
        assert_eq!(switched.name, "保留名称");
        assert_eq!(switched.controls.blur, 0);
        assert_eq!(switched.asset_id, d.asset_id);
        assert_eq!(switched.design.as_ref().unwrap().background_x, 23);
        assert_eq!(switched.design.as_ref().unwrap().background_y, 71);
        assert!(switched.manual_overrides.regions.is_empty());
        assert!(!switched.manual_overrides.global);
        assert_eq!(store.read_revision(&original).unwrap().compiled.css, css);
        assert_eq!(store.list().unwrap().len(), 1);
    }
    #[test]
    fn old_advice_and_css_only_migrate_when_a_profile_is_explicitly_adopted() {
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let mut d = store.import("test.png", &fixture()).unwrap().document;
        d.schema_version = 2;
        d.style = None;
        d.analysis = None;
        d.advice.accent = Some("#334455".into());
        d.compiled = region_theme::compile_controls(
            theme_engine::compile(&d.background, &d.extracted_accent, &d.controls, &d.advice)
                .unwrap(),
            d.design.as_ref().unwrap(),
            &d.background_samples,
            &d.controls,
        )
        .unwrap();
        store.write_draft(&d).unwrap();
        let reference = store.save(&d.draft_id).unwrap();
        let opened = store.open(&reference).unwrap().document;
        assert_eq!(opened.schema_version, 2);
        assert_eq!(opened.compiled.css, d.compiled.css);
        assert_eq!(opened.advice.accent, d.advice.accent);
        let changed = store
            .update(
                &opened.draft_id,
                DraftUpdate {
                    name: opened.name.clone(),
                    controls: opened.controls.clone(),
                    style: Some(theme_compiler::StyleSelection {
                        id: theme_compiler::StyleId::AiryLight,
                        version: 1,
                    }),
                    sequence: 1,
                    ..Default::default()
                },
            )
            .unwrap()
            .document;
        assert_eq!(changed.schema_version, 3);
        assert_eq!(
            store.read_revision(&reference).unwrap().compiled.css,
            d.compiled.css
        );
    }
    #[test]
    #[cfg(windows)]
    fn a_failed_edit_never_advances_its_sequence_or_loses_the_last_input() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let doc = store.import("test.png", &fixture()).unwrap().document;
        let blocked = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(store.draft_path(&doc.draft_id).unwrap())
            .unwrap();
        let update = DraftUpdate {
            name: "retry me".into(),
            controls: doc.controls.clone(),
            sequence: 1,
            ..Default::default()
        };
        assert!(store.update(&doc.draft_id, update.clone()).is_err());
        assert_eq!(store.read_draft(&doc.draft_id).unwrap().edit_sequence, 0);
        drop(blocked);
        let result = store.update(&doc.draft_id, update).unwrap().document;
        assert_eq!(result.name, "retry me");
        assert_eq!(result.edit_sequence, 1);
        assert_eq!(store.latest().unwrap().unwrap().document.edit_sequence, 1);
    }
    #[test]
    fn new_imports_are_versioned_offline_documents() {
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let doc = store.import("test.png", &fixture()).unwrap().document;
        assert_eq!(doc.schema_version, 3);
        let json = serde_json::to_value(&doc).unwrap();
        assert_eq!(json["style"]["version"], 2);
        assert_eq!(json["analysis"]["version"], 2);
        assert_eq!(json["analysis"]["samples"].as_array().unwrap().len(), 256);
        assert_eq!(doc.controls.blur, 0);
        assert_eq!(store.list().unwrap().len(), 0);
    }
    #[test]
    fn old_revision_keeps_its_css_and_upgraded_design_is_a_new_revision() {
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let id = store
            .import("test.png", &fixture())
            .unwrap()
            .document
            .draft_id;
        let mut old = store.read_draft(&id).unwrap();
        old.schema_version = 1;
        old.design = None;
        old.background_samples.clear();
        old.compiled = theme_engine::compile(
            &old.background,
            &old.extracted_accent,
            &old.controls,
            &old.advice,
        )
        .unwrap();
        store.write_draft(&old).unwrap();
        let reference = store.save(&id).unwrap();
        let old_css = old.compiled.css.clone();
        let opened = store.open(&reference).unwrap().document;
        store
            .update_design(&opened.draft_id, opened.edit_sequence, None)
            .unwrap();
        let next = store.save(&opened.draft_id).unwrap();
        assert_ne!(reference, next);
        assert_eq!(
            store.read_revision(&reference).unwrap().compiled.css,
            old_css
        );
        assert_eq!(store.read_revision(&next).unwrap().schema_version, 2);
    }
    #[test]
    fn manual_accent_changes_regional_buttons_without_an_ai_request() {
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let view = store.import("test.png", &fixture()).unwrap();
        let updated = store
            .update(
                &view.document.draft_id,
                DraftUpdate {
                    name: "按钮".into(),
                    sequence: 1,
                    controls: ThemeControls {
                        accent: Some("#CC2255".into()),
                        ..ThemeControls::default()
                    },
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(updated.document.compiled.css.contains("rgb(204 34 85"));
        assert!(store.list().unwrap().is_empty());
    }
    #[test]
    fn incomplete_publication_does_not_reuse_a_revision() {
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let id = store
            .import("test.png", &fixture())
            .unwrap()
            .document
            .draft_id;
        let first = store.save(&id).unwrap();
        fs::create_dir(store.theme_dir(&first.id).unwrap().join("r2")).unwrap();
        store
            .update(
                &id,
                DraftUpdate {
                    name: "after crash".into(),
                    controls: ThemeControls::default(),
                    sequence: 1,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(store.save(&id).unwrap().revision, Some(3));
        assert_eq!(store.list().unwrap().len(), 1);
        assert!(store.read_revision(&first).is_ok());
    }
    fn fixture() -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            64,
            64,
            image::Rgb([150, 180, 170]),
        ))
        .write_to(&mut cursor, image::ImageFormat::Png)
        .unwrap();
        cursor.into_inner()
    }
    #[test]
    fn draft_save_reopen_revision_and_trash_roundtrip() {
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().join("studio"),
        };
        let view = store.import("test.png", &fixture()).unwrap();
        let id = view.document.draft_id;
        assert!(store.list().unwrap().is_empty());
        let saved = store.save(&id).unwrap();
        assert_eq!(store.save(&id).unwrap(), saved);
        assert_eq!(store.list().unwrap().len(), 1);
        let opened = store.open(&saved).unwrap();
        store
            .update(
                &opened.document.draft_id,
                DraftUpdate {
                    name: "新名称".into(),
                    controls: ThemeControls {
                        brightness: 12,
                        ..ThemeControls::default()
                    },
                    sequence: 1,
                    ..Default::default()
                },
            )
            .unwrap();
        let next = store.save(&opened.document.draft_id).unwrap();
        assert_eq!(next.revision, Some(2));
        assert_eq!(store.read_revision(&saved).unwrap().name, "我的图片主题");
        assert_eq!(store.latest().unwrap().unwrap().document.name, "新名称");
        store.trash(&saved.id).unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(temp.path().join("studio/trash").is_dir());
    }
    #[test]
    fn stale_updates_cannot_overwrite_newer_draft() {
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let id = store
            .import("test.png", &fixture())
            .unwrap()
            .document
            .draft_id;
        for (seq, name) in [(2, "新"), (1, "旧")] {
            store
                .update(
                    &id,
                    DraftUpdate {
                        name: name.into(),
                        controls: ThemeControls::default(),
                        sequence: seq,
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        assert_eq!(store.read_draft(&id).unwrap().name, "新");
        assert!(store.list().unwrap().is_empty());
    }
    #[test]
    fn unsafe_ids_and_failed_saves_never_publish() {
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        assert!(store.read_draft("../state").is_err());
        assert!(store.trash("../outside").is_err());
        let id = store
            .import("test.png", &fixture())
            .unwrap()
            .document
            .draft_id;
        let doc = store.read_draft(&id).unwrap();
        fs::remove_file(store.asset_file(&doc.asset_id, "hero.jpg").unwrap()).unwrap();
        assert!(store.save(&id).is_err());
        assert!(store.list().unwrap().is_empty());
    }
    #[test]
    #[cfg(windows)]
    fn failed_draft_write_does_not_publish_a_new_library_head() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let draft = store.import("test.png", &fixture()).unwrap().document;
        let original = store.save(&draft.draft_id).unwrap();
        store
            .update(
                &draft.draft_id,
                DraftUpdate {
                    name: "uncommitted rename".into(),
                    controls: ThemeControls::default(),
                    sequence: 1,
                    ..Default::default()
                },
            )
            .unwrap();
        // Allow the save to read the draft but deny replacing it, like an antivirus/editor lock.
        let _blocked = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(store.draft_path(&draft.draft_id).unwrap())
            .unwrap();
        assert!(store.save(&draft.draft_id).is_err());
        let visible = store.list().unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(
            visible[0].reference, original,
            "a reported save failure must keep the previously published head"
        );
    }
    #[test]
    fn library_rename_never_replaces_an_unrelated_unsaved_latest_draft() {
        let mut preserved = Vec::new();
        for name in ["renamed library theme", ""] {
            let temp = tempfile::tempdir().unwrap();
            let store = ThemeStore {
                root: temp.path().into(),
            };
            let saved_id = store
                .import("saved.png", &fixture())
                .unwrap()
                .document
                .draft_id;
            let reference = store.save(&saved_id).unwrap();
            let unsaved = store.import("unsaved.png", &fixture()).unwrap().document;
            assert_eq!(
                store.latest().unwrap().unwrap().document.draft_id,
                unsaved.draft_id
            );
            let result = store.rename(&reference, name);
            assert_eq!(result.is_ok(), !name.is_empty());
            if let Ok(renamed) = result {
                assert_eq!(store.read_revision(&renamed).unwrap().name, name);
                assert_eq!(store.list().unwrap()[0].name, name);
            }
            assert!(
                store.read_draft(&unsaved.draft_id).is_ok(),
                "the unsaved file still exists but must also remain discoverable"
            );
            preserved.push(store.latest().unwrap().unwrap().document.draft_id == unsaved.draft_id);
        }
        assert_eq!(
            preserved,
            vec![true, true],
            "neither successful nor rejected library rename may replace another editing session"
        );
    }
    #[test]
    #[cfg(windows)]
    fn failed_head_commit_rolls_back_draft_and_retry_never_returns_an_orphan() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let id = store
            .import("test.png", &fixture())
            .unwrap()
            .document
            .draft_id;
        let first = store.save(&id).unwrap();
        store
            .update(
                &id,
                DraftUpdate {
                    name: "second".into(),
                    controls: ThemeControls::default(),
                    sequence: 1,
                    ..Default::default()
                },
            )
            .unwrap();
        let before = fs::read(store.draft_path(&id).unwrap()).unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(store.theme_dir(&first.id).unwrap().join("head.json"))
            .unwrap();
        assert!(store.save(&id).is_err());
        assert_eq!(store.list().unwrap()[0].reference, first);
        assert_eq!(fs::read(store.draft_path(&id).unwrap()).unwrap(), before);
        drop(lock);
        let retried = store.save(&id).unwrap();
        assert_eq!(retried.revision, Some(3));
        assert_eq!(store.save(&id).unwrap(), retried);
        let orphan = ThemeRef {
            id: first.id,
            revision: Some(2),
        };
        assert!(store.published_revision(&orphan).is_err());
        let orphan_doc = store.read_revision(&orphan).unwrap();
        write_json(&store.draft_path(&id).unwrap(), &orphan_doc).unwrap();
        assert!(
            store.save(&id).is_err(),
            "saved_sequence alone must not return an unpublished revision"
        );
    }
    #[test]
    fn interrupted_save_recovers_both_sides_of_the_head_commit() {
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let id = store
            .import("test.png", &fixture())
            .unwrap()
            .document
            .draft_id;
        let first = store.save(&id).unwrap();
        let before = store
            .update(
                &id,
                DraftUpdate {
                    name: "next".into(),
                    controls: ThemeControls::default(),
                    sequence: 1,
                    ..Default::default()
                },
            )
            .unwrap()
            .document;
        let next = store.publish_document(before.clone(), None).unwrap();
        let after = store.read_revision(&next).unwrap();
        let transaction = PendingSave {
            before: before.clone(),
            reference: next.clone(),
            previous_head: first.revision,
        };
        // Crash after draft metadata changed but before the new head was committed.
        write_json(
            &store.theme_dir(&first.id).unwrap().join("head.json"),
            &first.revision.unwrap(),
        )
        .unwrap();
        write_json(&store.pending_save_path(&id).unwrap(), &transaction).unwrap();
        write_json(&store.draft_path(&id).unwrap(), &after).unwrap();
        assert_eq!(
            serde_json::to_value(store.read_draft(&id).unwrap()).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
        assert_eq!(store.list().unwrap()[0].reference, first);
        // Crash after head committed, with a leftover transaction and stale draft metadata.
        write_json(
            &store.theme_dir(&first.id).unwrap().join("head.json"),
            &next.revision.unwrap(),
        )
        .unwrap();
        write_json(&store.pending_save_path(&id).unwrap(), &transaction).unwrap();
        assert_eq!(
            serde_json::to_value(store.read_draft(&id).unwrap()).unwrap(),
            serde_json::to_value(&after).unwrap()
        );
        assert_eq!(store.save(&id).unwrap(), next);
        assert!(!store.pending_save_path(&id).unwrap().exists());
    }
    #[test]
    fn mismatched_save_journal_never_overwrites_the_requested_draft() {
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let id = store
            .import("test.png", &fixture())
            .unwrap()
            .document
            .draft_id;
        let reference = store.save(&id).unwrap();
        let mut before = store.read_draft(&id).unwrap();
        let original = fs::read(store.draft_path(&id).unwrap()).unwrap();
        before.draft_id = "a".repeat(32);
        write_json(
            &store.pending_save_path(&id).unwrap(),
            &PendingSave {
                before,
                reference,
                previous_head: None,
            },
        )
        .unwrap();
        assert!(store.read_draft(&id).is_err());
        assert_eq!(fs::read(store.draft_path(&id).unwrap()).unwrap(), original);
    }
    #[test]
    fn unchanged_historical_revision_save_stays_pinned_and_does_not_republish() {
        let temp = tempfile::tempdir().unwrap();
        let store = ThemeStore {
            root: temp.path().into(),
        };
        let id = store
            .import("test.png", &fixture())
            .unwrap()
            .document
            .draft_id;
        let first = store.save(&id).unwrap();
        let next = store.rename(&first, "new name").unwrap();
        let opened = store.open(&first).unwrap().document;
        assert_eq!(store.save(&opened.draft_id).unwrap(), first);
        assert_eq!(store.save(&opened.draft_id).unwrap(), first);
        assert_eq!(store.list().unwrap()[0].reference, next);
    }
}
