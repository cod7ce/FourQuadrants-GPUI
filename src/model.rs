//! Pure-Rust domain model for FourQuadrants GPUI.
//!
//! The model intentionally has no persistence or UI dependencies. JSON uses
//! camelCase keys and every field has a serde default so that a native export
//! containing `"settings": {}` (or older partial task objects) can still load.

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, Datelike, Days, Local, NaiveDate, TimeZone, Utc};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use uuid::Uuid;

/// Latest supported on-disk schema version. Markdown notes require version 2.
pub const SCHEMA_VERSION: u32 = 2;

/// Maximum decoded size of one note attachment (20 MiB).
pub const MAX_NOTE_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;

/// Maximum combined decoded attachment size per Markdown note (50 MiB).
pub const MAX_NOTE_ATTACHMENTS_BYTES: usize = 50 * 1024 * 1024;

/// Maximum task nesting: top level = 1, child = 2, grandchild = 3.
pub const MAX_TASK_DEPTH: usize = 3;

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

fn utc_now() -> DateTime<Utc> {
    Utc::now()
}

fn current_week_start() -> NaiveDate {
    week_start(today())
}

/// Today's date in the machine's local timezone.
pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Normalizes a date to the Monday that starts its week.
pub fn week_start(date: NaiveDate) -> NaiveDate {
    let offset = date.weekday().num_days_from_monday() as u64;
    date.checked_sub_days(Days::new(offset)).unwrap_or(date)
}

/// Maps a quadrant index to `(urgent, important)`.
///
/// 0 = urgent + important, 1 = important only, 2 = urgent only,
/// 3 = neither. Values outside 0..=3 are treated as quadrant 3.
pub fn quadrant_flags(quadrant: usize) -> (bool, bool) {
    match quadrant {
        0 => (true, true),
        1 => (false, true),
        2 => (true, false),
        _ => (false, false),
    }
}

// ---------------------------------------------------------------------------
// Model types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Document {
    pub schema_version: u32,
    pub tasks: Vec<Task>,
    pub tags: Vec<Tag>,
    pub week_notes: Vec<WeekNote>,
    /// Conflicting versions of a week note (same week, different content).
    /// The canonical version stays in `week_notes`; this preserves imports for
    /// later UI reconciliation. Serialized as `archivedWeekNotes`.
    pub archived_week_notes: Vec<WeekNote>,
    /// Progress notes whose task disappeared in a legacy store. These are
    /// preserved instead of being silently discarded.
    pub orphan_progress_notes: Vec<ProgressNote>,
    pub settings: Settings,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            // Keep plain documents compatible with schema 1 until Markdown is saved.
            schema_version: 1,
            tasks: Vec::new(),
            tags: Vec::new(),
            week_notes: Vec::new(),
            archived_week_notes: Vec::new(),
            orphan_progress_notes: Vec::new(),
            settings: Settings::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub title: String,
    pub notes: String,
    pub urgent: bool,
    pub important: bool,
    pub completed: bool,
    pub completed_at: Option<DateTime<Utc>>,
    pub due_date: Option<DateTime<Utc>>,
    pub remind_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub sort_order: f64,
    pub week_start: NaiveDate,
    pub parent_id: Option<String>,
    pub tags: Vec<String>,
    pub links: Vec<String>,
    pub issue_key: Option<String>,
    /// Base64-encoded RTFD payload.
    pub rich_content: Option<String>,
    pub progress_notes: Vec<ProgressNote>,
}

impl Default for Task {
    fn default() -> Self {
        Self {
            id: new_id(),
            title: String::new(),
            notes: String::new(),
            urgent: false,
            important: false,
            completed: false,
            completed_at: None,
            due_date: None,
            remind_at: None,
            created_at: utc_now(),
            sort_order: 0.0,
            week_start: current_week_start(),
            parent_id: None,
            tags: Vec::new(),
            links: Vec::new(),
            issue_key: None,
            rich_content: None,
            progress_notes: Vec::new(),
        }
    }
}

impl Task {
    /// 0 = urgent + important, 1 = important, 2 = urgent, 3 = neither.
    pub fn quadrant(&self) -> usize {
        match (self.urgent, self.important) {
            (true, true) => 0,
            (false, true) => 1,
            (true, false) => 2,
            (false, false) => 3,
        }
    }

    /// Completed tasks use their completion date; open tasks use their due date.
    /// The original falls back to the due date when `completedAt` is missing.
    /// Timestamps are stored in UTC but interpreted on the local calendar.
    pub fn agenda_date(&self) -> Option<NaiveDate> {
        let value = if self.completed {
            self.completed_at.as_ref().or(self.due_date.as_ref())
        } else {
            self.due_date.as_ref()
        };
        value.map(|dt| dt.with_timezone(&Local).date_naive())
    }

    /// Returns the newest progress note first.
    pub fn latest_progress_note(&self) -> Option<&ProgressNote> {
        self.progress_notes
            .iter()
            .max_by(|a, b| a.created_at.cmp(&b.created_at))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ProgressNote {
    pub id: String,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

impl Default for ProgressNote {
    fn default() -> Self {
        Self {
            id: new_id(),
            text: String::new(),
            created_at: utc_now(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color_hex: String,
    pub sort_order: f64,
}

impl Default for Tag {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: String::new(),
            color_hex: "#1E88E5".to_string(),
            sort_order: 0.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct WeekNote {
    pub week_start: NaiveDate,
    /// Original plain text, preserved verbatim when Markdown is edited.
    pub text: String,
    /// Original base64-encoded RTFD payload, preserved verbatim.
    pub rich_content: Option<String>,
    /// Separate Markdown content. `Some` also represents an intentional clear.
    pub markdown: Option<MarkdownNote>,
}

impl Default for WeekNote {
    fn default() -> Self {
        Self {
            week_start: current_week_start(),
            text: String::new(),
            rich_content: None,
            markdown: None,
        }
    }
}

impl WeekNote {
    /// Markdown takes precedence, including an intentionally empty source.
    pub fn editable_text(&self) -> &str {
        self.markdown
            .as_ref()
            .map_or(self.text.as_str(), |markdown| markdown.source.as_str())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct MarkdownNote {
    pub source: String,
    pub attachments: Vec<NoteAttachment>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct NoteAttachment {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    /// Base64-encoded original bytes; never recompressed or normalized on save.
    pub data: String,
}

impl Default for NoteAttachment {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: String::new(),
            mime_type: "application/octet-stream".to_string(),
            data: String::new(),
        }
    }
}

impl MarkdownNote {
    /// Validates attachment identities, supported MIME types and decoded sizes.
    /// Attachment IDs are unique within this note, including alternate UUID spellings.
    pub fn validate(&self) -> Result<()> {
        let mut ids = HashSet::new();
        let mut total_bytes = 0usize;
        for attachment in &self.attachments {
            let label = format!("attachment {:?} ({})", attachment.name, attachment.id);
            let id = Uuid::parse_str(&attachment.id)
                .with_context(|| format!("{label} must have a valid UUID id"))?;
            ensure!(ids.insert(id), "duplicate attachment id {}", attachment.id);
            ensure!(
                matches!(
                    attachment.mime_type.as_str(),
                    "image/png"
                        | "image/jpeg"
                        | "image/webp"
                        | "image/gif"
                        | "image/bmp"
                        | "image/tiff"
                        | "application/octet-stream"
                ),
                "{label} has unsupported MIME type {:?}",
                attachment.mime_type
            );
            // Reject oversized input before allocating a decoded buffer.
            ensure!(
                attachment.data.len() <= MAX_NOTE_ATTACHMENT_BYTES.div_ceil(3) * 4,
                "{label} exceeds the 20 MiB attachment limit"
            );
            let bytes = BASE64
                .decode(&attachment.data)
                .with_context(|| format!("{label} contains invalid base64 data"))?;
            ensure!(!bytes.is_empty(), "{label} must contain nonempty data");
            ensure!(
                bytes.len() <= MAX_NOTE_ATTACHMENT_BYTES,
                "{label} exceeds the 20 MiB attachment limit"
            );
            total_bytes += bytes.len();
            ensure!(
                total_bytes <= MAX_NOTE_ATTACHMENTS_BYTES,
                "note attachments exceed the 50 MiB total limit at {label}"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// system | light | dark | ocean | forest | terminal
    pub theme: String,
    /// None preserves the original named palettes until appearance is edited.
    pub appearance: Option<AppearanceSettings>,
    /// zh | en
    pub language: String,
    pub font_size: f32,
    pub font_family: String,
    pub show_inbox: bool,
    pub hide_completed: bool,
    pub task_confetti: bool,
    pub task_completion_sound: bool,
    /// The sidebar uses a compact icon rail to leave more room for content.
    pub sidebar_collapsed: bool,
    pub glass: bool,
    /// Theme tint strength over the native glass material, 0..=1.
    pub opacity: f32,
    /// None preserves the legacy glass settings until window effects are edited.
    pub window_effects: Option<WindowEffectsSettings>,
    pub background_image: Option<String>,
    pub background_opacity: f32,
    pub background_blur: f32,
    /// Darkening scrim over the background image, 0..=1.
    pub background_scrim: f32,
    /// fill | fit | tile | stretch
    pub background_fill: String,
    pub parse_rules: Vec<ParseRule>,
    /// Tag applied by the built-in smart parser when no rule matched.
    pub builtin_parse_tag_name: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".to_string(),
            appearance: None,
            language: "zh".to_string(),
            font_size: 14.0,
            font_family: String::new(),
            show_inbox: true,
            hide_completed: false,
            task_confetti: true,
            task_completion_sound: true,
            sidebar_collapsed: false,
            glass: true,
            opacity: 0.50,
            window_effects: None,
            background_image: None,
            background_opacity: 1.0,
            background_blur: 0.0,
            background_scrim: 0.35,
            background_fill: "fill".to_string(),
            parse_rules: Vec::new(),
            builtin_parse_tag_name: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct WindowEffectsSettings {
    /// Window transparency, 0..=1.
    pub transparency: f32,
    /// Blur radius behind the window, 0..=64; independent of background images.
    pub blur_radius: f32,
}

impl Default for WindowEffectsSettings {
    fn default() -> Self {
        Self {
            transparency: 0.40,
            blur_radius: 24.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct AppearanceSettings {
    /// Surface hue in degrees, 0..=360.
    pub hue: f32,
    /// Surface saturation, 0..=1.
    pub saturation: f32,
    /// Dark background lightness, 0..=0.20.
    pub dark_lightness: f32,
    /// Opaque accent color in #RRGGBB notation.
    pub accent_hex: String,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            hue: 220.0,
            saturation: 0.10,
            dark_lightness: 0.05,
            accent_hex: "#6AA9EF".to_string(),
        }
    }
}

impl Settings {
    /// Resolves legacy glass settings without changing the serialized settings.
    pub fn window_effects_values(&self, dark: bool) -> WindowEffectsSettings {
        if let Some(effects) = &self.window_effects {
            return effects.clone();
        }
        if !self.glass {
            return WindowEffectsSettings {
                transparency: 0.0,
                blur_radius: 0.0,
            };
        }
        let opacity = self.opacity.clamp(0.0, 1.0);
        let root_alpha = if dark {
            0.30 + opacity * 0.50
        } else {
            0.08 + opacity * 0.28
        };
        WindowEffectsSettings {
            // Compose the legacy root tint and the panel's 0.12 alpha.
            transparency: (1.0 - root_alpha) * (1.0 - 0.12),
            blur_radius: 24.0,
        }
    }

    /// Materializes window effects on the first edit, preserving other settings.
    pub fn customize_window_effects(&mut self, dark: bool) -> &mut WindowEffectsSettings {
        if self.window_effects.is_none() {
            self.window_effects = Some(self.window_effects_values(dark));
        }
        self.window_effects
            .as_mut()
            .expect("window effects were materialized")
    }

    /// Resolves old named themes without changing the serialized settings.
    pub fn appearance_values(&self) -> AppearanceSettings {
        if let Some(appearance) = &self.appearance {
            return appearance.clone();
        }
        // HSL approximations of the legacy backgrounds; accents stay exact.
        let (hue, saturation, dark_lightness, accent_hex) = match self.theme.as_str() {
            "ocean" => (212.0, 0.35, 0.076, "#7DB8D9"),
            "forest" => (146.0, 0.20, 0.065, "#A3C1A3"),
            "terminal" => (140.0, 0.23, 0.044, "#8FC9A0"),
            _ => return AppearanceSettings::default(),
        };
        AppearanceSettings {
            hue,
            saturation,
            dark_lightness,
            accent_hex: accent_hex.to_string(),
        }
    }

    /// The three display modes; legacy named themes always used dark mode.
    pub fn theme_mode(&self) -> &str {
        match self.theme.as_str() {
            "ocean" | "forest" | "terminal" => "dark",
            mode => mode,
        }
    }

    /// Call before any appearance edit, including changing mode or resetting.
    pub fn customize_appearance(&mut self) -> &mut AppearanceSettings {
        if self.appearance.is_none() {
            self.appearance = Some(self.appearance_values());
        }
        if self.theme == "terminal" {
            // Terminal forced Menlo even when another font had been saved.
            self.font_family = "Menlo".to_string();
        }
        if matches!(self.theme.as_str(), "ocean" | "forest" | "terminal") {
            self.theme = "dark".to_string();
        }
        self.appearance
            .as_mut()
            .expect("appearance was materialized")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ParseRule {
    pub id: String,
    pub name: String,
    pub pattern: String,
    /// Capture group for the issue key; 0 means none.
    pub issue_group: usize,
    /// Capture group for the title; 0 means empty.
    pub title_group: usize,
    /// Capture group for one link; 0 means none.
    pub link_group: usize,
    /// Tag name applied when this rule matches; empty means none.
    pub tag_name: String,
}

impl Default for ParseRule {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: String::new(),
            pattern: String::new(),
            issue_group: 0,
            title_group: 0,
            link_group: 0,
            tag_name: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Paste parsing (mirrors Swift ParseEngine / TaskInputParser)
// ---------------------------------------------------------------------------

/// Result of parsing pasted text. All fields are public for direct UI binding.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct ParsedTaskInput {
    pub issue_key: Option<String>,
    pub title: String,
    pub links: Vec<String>,
    pub tag_name: Option<String>,
}

impl ParsedTaskInput {
    pub fn is_empty(&self) -> bool {
        self.issue_key.is_none() && self.title.is_empty() && self.links.is_empty()
    }
}

impl ParseRule {
    /// Applies this rule to `text`. Invalid regular expressions and
    /// non-matching rules return `None`, matching the Swift implementation.
    pub fn apply(&self, text: &str) -> Option<ParsedTaskInput> {
        if self.pattern.is_empty() {
            return None;
        }
        let regex = RegexBuilder::new(&self.pattern)
            .dot_matches_new_line(true)
            .build()
            .ok()?;
        let captures = regex.captures(text)?;

        let group = |index: usize| -> Option<String> {
            if index == 0 {
                return None;
            }
            captures
                .get(index)
                .map(|value| value.as_str().trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let issue = group(self.issue_group);
        let title = group(self.title_group).unwrap_or_default();
        let link = group(self.link_group);
        let tag = if self.tag_name.trim().is_empty() {
            None
        } else {
            Some(self.tag_name.trim().to_string())
        };

        Some(ParsedTaskInput {
            issue_key: issue,
            title,
            links: link.into_iter().collect(),
            tag_name: tag,
        })
    }
}

impl Settings {
    /// Applies the first non-empty user rule, then falls back to the built-in
    /// smart parser and finally to `builtinParseTagName`.
    pub fn parse_input(&self, raw: &str) -> ParsedTaskInput {
        for rule in &self.parse_rules {
            if let Some(result) = rule.apply(raw)
                && !result.is_empty()
            {
                return result;
            }
        }

        let mut result = parse_task_input(raw);
        if result.tag_name.is_none() {
            let tag = self.builtin_parse_tag_name.trim();
            if !tag.is_empty() {
                result.tag_name = Some(tag.to_string());
            }
        }
        result
    }
}

fn url_regex() -> &'static Regex {
    static URL_RE: OnceLock<Regex> = OnceLock::new();
    URL_RE.get_or_init(|| {
        Regex::new(r#"(?i)\b(?:https?://|www\.)[^\s<>"']+"#)
            .expect("built-in URL regex must compile")
    })
}

fn issue_regex() -> &'static Regex {
    static ISSUE_RE: OnceLock<Regex> = OnceLock::new();
    ISSUE_RE.get_or_init(|| {
        Regex::new(r"\b[A-Z][A-Z0-9]*-\d+\b").expect("built-in issue regex must compile")
    })
}

fn whitespace_regex() -> &'static Regex {
    static WHITESPACE_RE: OnceLock<Regex> = OnceLock::new();
    WHITESPACE_RE
        .get_or_init(|| Regex::new(r"\s+").expect("built-in whitespace regex must compile"))
}

fn trim_url(url: &str) -> &str {
    url.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}'])
}

/// Built-in smart parsing: URLs, a leading issue key, then a whitespace-folded
/// title made from the remaining text.
pub fn parse_task_input(raw: &str) -> ParsedTaskInput {
    let mut links = Vec::new();
    let without_links = url_regex()
        .replace_all(raw, |caps: &regex::Captures<'_>| {
            let full = caps.get(0).expect("whole URL match").as_str();
            let trimmed = trim_url(full);
            if !trimmed.is_empty() {
                links.push(trimmed.to_string());
            }
            " ".to_string()
        })
        .to_string();

    let mut text = without_links.trim().to_string();
    let mut issue_key = None;
    if let Some(found) = issue_regex().find(&text)
        && found.start() == 0
    {
        issue_key = Some(found.as_str().to_string());
        text = format!(" {}", &text[found.end()..]);
    }

    let title = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let title = whitespace_regex()
        .replace_all(&title, " ")
        .trim()
        .to_string();

    ParsedTaskInput {
        issue_key,
        title,
        links,
        tag_name: None,
    }
}

// ---------------------------------------------------------------------------
// Document operations
// ---------------------------------------------------------------------------

impl Document {
    pub fn task(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|task| task.id == id)
    }

    pub fn task_mut(&mut self, id: &str) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|task| task.id == id)
    }

    /// Direct children, ordered by `sortOrder` and then id for deterministic
    /// behavior when sort orders tie.
    pub fn children(&self, parent: &str) -> Vec<&Task> {
        let mut children: Vec<&Task> = self
            .tasks
            .iter()
            .filter(|task| task.parent_id.as_deref() == Some(parent))
            .collect();
        children.sort_by(|a, b| compare_tasks(a, b));
        children
    }

    /// Adds a top-level task in the requested week/quadrant and returns its
    /// stable id. A new task is appended after existing tasks in that group.
    pub fn add_task(&mut self, title: &str, week: NaiveDate, quadrant: usize) -> String {
        let week = week_start(week);
        let (urgent, important) = quadrant_flags(quadrant);
        let next_order = self
            .tasks
            .iter()
            .filter(|task| {
                task.parent_id.is_none()
                    && task.week_start == week
                    && task.urgent == urgent
                    && task.important == important
            })
            .map(|task| task.sort_order)
            .fold(-1.0_f64, f64::max)
            + 1.0;

        let task = Task {
            title: title.to_string(),
            urgent,
            important,
            sort_order: next_order,
            week_start: week,
            ..Task::default()
        };
        let id = task.id.clone();
        self.tasks.push(task);
        id
    }

    /// Adds a child with a one-time copy of its parent's tags and quadrant.
    /// The child starts in the parent's week and is appended after its siblings.
    /// Validate before inserting so a failed creation never leaves an orphan task.
    pub fn add_subtask(&mut self, title: &str, parent: &str) -> Result<String> {
        ensure!(
            self.depth_of(parent)? < MAX_TASK_DEPTH,
            "adding a subtask would exceed the maximum task depth of {MAX_TASK_DEPTH}"
        );
        let parent = self.task(parent).expect("parent was checked by depth_of");
        let task = Task {
            title: title.to_string(),
            urgent: parent.urgent,
            important: parent.important,
            week_start: parent.week_start,
            parent_id: Some(parent.id.clone()),
            tags: parent.tags.clone(),
            sort_order: self.next_sort_order(Some(&parent.id)),
            ..Task::default()
        };
        let id = task.id.clone();
        self.tasks.push(task);
        Ok(id)
    }

    /// Toggles exactly one task. The original does not recursively toggle the
    /// subtree, so children keep their own completion state.
    pub fn toggle_complete(&mut self, id: &str) {
        let now = utc_now();
        if let Some(task) = self.task_mut(id) {
            task.completed = !task.completed;
            task.completed_at = if task.completed { Some(now) } else { None };
        }
    }

    /// Deletes a task and its entire subtree, mirroring SwiftData's cascade
    /// delete rule. Safe on malformed cyclic data.
    pub fn delete_task(&mut self, id: &str) {
        if self.task(id).is_none() {
            return;
        }
        let remove: HashSet<String> = self.collect_subtree_ids(id).into_iter().collect();
        self.tasks.retain(|task| !remove.contains(&task.id));
    }

    /// Moves a task (and all descendants) to a week and quadrant. The task
    /// keeps its parent; use `reparent(..., None)` to promote it to top level.
    pub fn move_task(&mut self, id: &str, week: NaiveDate, quadrant: usize) {
        if self.task(id).is_none() {
            return;
        }
        let week = week_start(week);
        let (urgent, important) = quadrant_flags(quadrant);
        let subtree = self.collect_subtree_ids(id);
        for task_id in subtree {
            if let Some(task) = self.task_mut(&task_id) {
                task.week_start = week;
                if task_id == id {
                    task.urgent = urgent;
                    task.important = important;
                }
            }
        }
    }

    /// Attaches a task below `parent` (or detaches it when `None`). The whole
    /// subtree follows the new parent's week, keeping its own tags and quadrants.
    /// Cycles and depth > 3 are errors.
    pub fn reparent(&mut self, id: &str, parent: Option<&str>) -> Result<()> {
        let target_idx = self
            .index_of(id)
            .ok_or_else(|| anyhow::anyhow!("task {id} not found"))?;

        if self.tasks[target_idx].parent_id.as_deref() == parent {
            return Ok(());
        }

        if let Some(parent_id) = parent {
            ensure!(parent_id != id, "a task cannot be its own parent");
            let parent_idx = self
                .index_of(parent_id)
                .ok_or_else(|| anyhow::anyhow!("parent task {parent_id} not found"))?;
            ensure!(
                !self.is_in_subtree(parent_id, id)?,
                "cannot reparent task {id} below its own descendant {parent_id}"
            );

            let parent_depth = self.depth_of(parent_id)?;
            let height = self.subtree_height(id)?;
            ensure!(
                parent_depth + height <= MAX_TASK_DEPTH,
                "reparenting {id} would exceed the maximum task depth of {MAX_TASK_DEPTH}"
            );

            let parent_week = self.tasks[parent_idx].week_start;
            let next_order = self.next_sort_order(Some(parent_id));
            self.tasks[target_idx].parent_id = Some(parent_id.to_string());
            self.tasks[target_idx].sort_order = next_order;
            self.set_subtree_week(id, parent_week);
        } else {
            let sort_order = self.next_sort_order(None);
            self.tasks[target_idx].parent_id = None;
            self.tasks[target_idx].sort_order = sort_order;
        }
        Ok(())
    }

    /// Schedules a task at 18:30 local time on the chosen day and moves the
    /// whole subtree into that day's week.
    pub fn schedule(&mut self, id: &str, date: NaiveDate) {
        let idx = match self.index_of(id) {
            Some(idx) => idx,
            None => return,
        };
        let naive = date.and_hms_opt(18, 30, 0).expect("18:30 is always valid");
        let due = Local
            .from_local_datetime(&naive)
            .earliest()
            .map(|value| value.with_timezone(&Utc))
            .unwrap_or_else(|| naive.and_utc());
        self.tasks[idx].due_date = Some(due);
        let week = week_start(date);
        self.tasks[idx].week_start = week;
        self.set_subtree_week(id, week);
    }

    /// Carries every unfinished top-level task from `from` into `to`, exactly
    /// matching the original three carry actions:
    ///
    /// * no children: move and clear due/reminder;
    /// * some unfinished child: copy the parent into the target week, move
    ///   unfinished direct children (and their subtrees) under the copy, then
    ///   complete the original;
    /// * all children finished: complete the original, leave it in place.
    ///
    /// Cross-week work is a no-op.
    pub fn carry_forward(&mut self, from: NaiveDate, to: NaiveDate) {
        let from = week_start(from);
        let to = week_start(to);
        if from >= to {
            return;
        }

        let mut source_ids: Vec<String> = self
            .tasks
            .iter()
            .filter(|task| task.parent_id.is_none() && !task.completed && task.week_start == from)
            .map(|task| task.id.clone())
            .collect();
        source_ids.sort_by(|a, b| {
            let ta = self.task(a);
            let tb = self.task(b);
            match (ta, tb) {
                (Some(ta), Some(tb)) => compare_tasks(ta, tb),
                _ => a.cmp(b),
            }
        });
        self.carry_selected(&source_ids, to);
    }

    /// Applies the same three carry actions to explicitly selected tasks,
    /// regardless of their source week. The UI uses this for the all-earlier-
    /// weeks selection dialog. Missing ids and duplicates are ignored; input
    /// order is preserved.
    pub fn carry_selected(&mut self, ids: &[String], to: NaiveDate) {
        let to = week_start(to);
        let mut seen = HashSet::new();
        for source_id in ids {
            if !seen.insert(source_id.as_str()) {
                continue;
            }
            self.carry_one(source_id, to);
        }
    }

    fn carry_one(&mut self, source_id: &str, to: NaiveDate) {
        let Some(source) = self.task(source_id).cloned() else {
            return;
        };
        let children = self.children(source_id);
        let has_children = !children.is_empty();
        let has_unfinished_children = children.iter().any(|child| !child.completed);

        if !has_children {
            if let Some(task) = self.task_mut(source_id) {
                task.week_start = to;
                task.due_date = None;
                task.remind_at = None;
            }
        } else if has_unfinished_children {
            let mut copy = Task {
                title: source.title.clone(),
                notes: source.notes.clone(),
                urgent: source.urgent,
                important: source.important,
                completed: false,
                completed_at: None,
                due_date: None,
                remind_at: None,
                sort_order: source.sort_order,
                week_start: to,
                parent_id: None,
                tags: source.tags.clone(),
                links: source.links.clone(),
                issue_key: source.issue_key.clone(),
                rich_content: source.rich_content.clone(),
                // The original copy action does not copy progress notes.
                progress_notes: Vec::new(),
                ..Task::default()
            };
            copy.created_at = utc_now();
            let copy_id = copy.id.clone();
            self.tasks.push(copy);

            let unfinished_children: Vec<String> = self
                .children(source_id)
                .into_iter()
                .filter(|child| !child.completed)
                .map(|child| child.id.clone())
                .collect();
            for child_id in unfinished_children {
                if let Some(child) = self.task_mut(&child_id) {
                    child.parent_id = Some(copy_id.clone());
                    child.due_date = None;
                    child.remind_at = None;
                }
                self.set_subtree_week(&child_id, to);
            }

            self.mark_completed_in_place(source_id);
        } else {
            self.mark_completed_in_place(source_id);
        }
    }

    /// Reorders `id` immediately before `before`. If `before` is a child, `id`
    /// becomes a sibling under the same parent; if `before` is top level, `id`
    /// becomes top level and adopts the target's week and quadrant. The new
    /// sibling group is reindexed from 0. Cycles and depth > 3 are errors.
    pub fn reorder(&mut self, id: &str, before: &str) -> Result<()> {
        self.validate_reorder(id, before)?;
        let dragged_idx = self.index_of(id).expect("validated source");
        let target_idx = self.index_of(before).expect("validated target");
        let new_parent = self.tasks[target_idx].parent_id.clone();
        let target_week = self.tasks[target_idx].week_start;
        let target_flags = (
            self.tasks[target_idx].urgent,
            self.tasks[target_idx].important,
        );

        if let Some(parent_id) = new_parent.clone() {
            let parent_week = self
                .task(&parent_id)
                .map(|task| task.week_start)
                .ok_or_else(|| anyhow::anyhow!("parent task {parent_id} not found"))?;
            self.tasks[dragged_idx].parent_id = Some(parent_id);
            self.set_subtree_week(id, parent_week);
        } else {
            self.tasks[dragged_idx].parent_id = None;
            self.tasks[dragged_idx].week_start = target_week;
            self.tasks[dragged_idx].urgent = target_flags.0;
            self.tasks[dragged_idx].important = target_flags.1;
            self.set_subtree_week(id, target_week);
        }

        let mut group: Vec<String> = if let Some(parent_id) = new_parent.as_deref() {
            self.children(parent_id)
                .into_iter()
                .map(|task| task.id.clone())
                .collect()
        } else {
            let mut tasks: Vec<&Task> = self
                .tasks
                .iter()
                .filter(|task| {
                    task.parent_id.is_none()
                        && task.week_start == target_week
                        && task.urgent == target_flags.0
                        && task.important == target_flags.1
                })
                .collect();
            tasks.sort_by(|a, b| compare_tasks(a, b));
            tasks.into_iter().map(|task| task.id.clone()).collect()
        };

        group.retain(|task_id| task_id != id);
        let insert_at = group
            .iter()
            .position(|task_id| task_id == before)
            .unwrap_or(group.len());
        group.insert(insert_at, id.to_string());

        for (order, task_id) in group.iter().enumerate() {
            if let Some(idx) = self.index_of(task_id) {
                self.tasks[idx].sort_order = order as f64;
            }
        }
        Ok(())
    }

    /// Read-only drop validation, shared with reorder so hover feedback cannot
    /// promise a move that would create a cycle or exceed the tree depth.
    pub fn validate_reorder(&self, id: &str, before: &str) -> Result<()> {
        ensure!(id != before, "cannot reorder a task before itself");
        ensure!(self.task(id).is_some(), "task {id} not found");
        let target = self
            .task(before)
            .ok_or_else(|| anyhow::anyhow!("task {before} not found"))?;
        if let Some(parent_id) = target.parent_id.as_deref() {
            ensure!(parent_id != id, "a task cannot be its own parent");
            ensure!(
                !self.is_in_subtree(parent_id, id)?,
                "cannot move task {id} below its own descendant {parent_id}"
            );
            let parent_depth = self.depth_of(parent_id)?;
            let height = self.subtree_height(id)?;
            ensure!(
                parent_depth + height <= MAX_TASK_DEPTH,
                "reordering {id} would exceed the maximum task depth of {MAX_TASK_DEPTH}"
            );
        }
        Ok(())
    }

    /// Merges an imported document into this one.
    ///
    /// * Non-conflicting ids are preserved unchanged.
    /// * An identical entity with the same id is skipped.
    /// * A colliding id with different content gets a deterministic UUID
    ///   derived from the source id and entity content. Re-importing the same
    ///   source therefore targets the same id again and is idempotent. Parent,
    ///   tag and nested reference ids are remapped consistently.
    /// * Local settings win. Schema 1 is promoted to 2 when Markdown is merged.
    /// * Same-week week-note conflicts are never discarded: the canonical note
    ///   remains in `week_notes`, and a distinct imported version is retained
    ///   in `archived_week_notes`. Exact duplicates are ignored.
    /// * Orphan progress notes are merged with the same stable-id rules.
    ///
    /// Returns the number of newly inserted tasks. The merge is transactional:
    /// on validation failure `self` is unchanged.
    pub fn merge(&mut self, other: Document) -> Result<usize> {
        self.validate()?;
        other.validate()?;

        let Document {
            tasks: other_tasks,
            tags: other_tags,
            week_notes: other_notes,
            archived_week_notes: other_archived_notes,
            orphan_progress_notes: other_orphans,
            ..
        } = other;

        let mut staged = self.clone();

        // Tags -----------------------------------------------------------------
        let existing_tags: HashMap<String, Tag> = staged
            .tags
            .iter()
            .cloned()
            .map(|tag| (tag.id.clone(), tag))
            .collect();
        let mut used_tag_ids: HashSet<String> = existing_tags.keys().cloned().collect();
        let mut tag_id_map: HashMap<String, String> = HashMap::new();
        let mut remapped_tag_ids: HashSet<String> = HashSet::new();
        let mut tags_to_add: Vec<Tag> = Vec::new();

        for mut tag in other_tags {
            let old_id = tag.id.clone();
            let existing = existing_tags.get(&old_id);
            if let Some(existing) = existing {
                if existing == &tag {
                    tag_id_map.insert(old_id.clone(), old_id);
                    continue;
                }
            } else if !used_tag_ids.contains(&old_id) {
                tag_id_map.insert(old_id.clone(), old_id.clone());
                used_tag_ids.insert(old_id);
                tags_to_add.push(tag);
                continue;
            }

            let stable_id = stable_import_id("tag", &old_id, &tag)?;
            tag_id_map.insert(old_id.clone(), stable_id.clone());
            remapped_tag_ids.insert(old_id);
            if used_tag_ids.contains(&stable_id) {
                // This exact imported entity was already merged previously.
                continue;
            }
            used_tag_ids.insert(stable_id.clone());
            tag.id = stable_id;
            tags_to_add.push(tag);
        }

        // Tasks ----------------------------------------------------------------
        let existing_tasks: HashMap<String, Task> = staged
            .tasks
            .iter()
            .cloned()
            .map(|task| (task.id.clone(), task))
            .collect();
        let mut used_task_ids: HashSet<String> = existing_tasks.keys().cloned().collect();
        let mut task_id_map: HashMap<String, String> = HashMap::new();
        let mut tasks_to_add: Vec<Task> = Vec::new();

        // A task needs a deterministic imported id when it collides, when one
        // of its tags was remapped, or when an ancestor was remapped. This
        // keeps the whole remapped branch stable across repeated imports.
        let mut exact_skips: HashSet<String> = HashSet::new();
        let mut needs_stable_task: HashSet<String> = HashSet::new();
        for task in &other_tasks {
            if let Some(existing) = existing_tasks.get(&task.id) {
                if existing == task {
                    exact_skips.insert(task.id.clone());
                    continue;
                }
                needs_stable_task.insert(task.id.clone());
            } else if used_task_ids.contains(&task.id) {
                needs_stable_task.insert(task.id.clone());
            }
            if task
                .tags
                .iter()
                .any(|tag_id| remapped_tag_ids.contains(tag_id))
            {
                needs_stable_task.insert(task.id.clone());
            }
        }
        loop {
            let mut changed = false;
            for task in &other_tasks {
                if exact_skips.contains(&task.id) || needs_stable_task.contains(&task.id) {
                    continue;
                }
                if task
                    .parent_id
                    .as_ref()
                    .is_some_and(|parent_id| needs_stable_task.contains(parent_id))
                {
                    needs_stable_task.insert(task.id.clone());
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        for mut task in other_tasks {
            let old_id = task.id.clone();
            if exact_skips.contains(&old_id) {
                task_id_map.insert(old_id.clone(), old_id);
                continue;
            }

            let stable = needs_stable_task.contains(&old_id);
            let final_id = if stable {
                stable_import_id("task", &old_id, &task)?
            } else {
                old_id.clone()
            };
            task_id_map.insert(old_id.clone(), final_id.clone());

            if stable {
                if used_task_ids.contains(&final_id) {
                    // Already imported by an earlier merge.
                    continue;
                }
            } else if used_task_ids.contains(&final_id) {
                // Extremely unlikely collision with a generated stable id;
                // preserve the entity under another deterministic id.
                let fallback = stable_import_id("task-collision", &old_id, &task)?;
                task_id_map.insert(old_id.clone(), fallback.clone());
                if used_task_ids.contains(&fallback) {
                    continue;
                }
                used_task_ids.insert(fallback.clone());
                task.id = fallback;
                tasks_to_add.push(task);
                continue;
            }

            used_task_ids.insert(final_id.clone());
            task.id = final_id;
            tasks_to_add.push(task);
        }

        for mut task in tasks_to_add {
            if let Some(parent_id) = task.parent_id.take() {
                let mapped = task_id_map.get(&parent_id).ok_or_else(|| {
                    anyhow::anyhow!(
                        "imported task {} references an unknown parent {parent_id}",
                        task.id
                    )
                })?;
                task.parent_id = Some(mapped.clone());
            }
            task.tags = task
                .tags
                .iter()
                .map(|tag_id| {
                    tag_id_map
                        .get(tag_id)
                        .cloned()
                        .unwrap_or_else(|| tag_id.clone())
                })
                .collect();
            staged.tasks.push(task);
        }
        staged.tags.extend(tags_to_add);

        // Orphan progress notes ------------------------------------------------
        for mut note in other_orphans {
            let old_id = note.id.clone();
            let existing = staged
                .orphan_progress_notes
                .iter()
                .find(|existing| existing.id == old_id);
            if let Some(existing) = existing {
                if existing == &note {
                    continue;
                }
            } else if !staged
                .orphan_progress_notes
                .iter()
                .any(|existing| existing.id == old_id)
            {
                staged.orphan_progress_notes.push(note);
                continue;
            }

            let stable_id = stable_import_id("orphan-progress-note", &old_id, &note)?;
            if staged
                .orphan_progress_notes
                .iter()
                .any(|existing| existing.id == stable_id)
            {
                continue;
            }
            note.id = stable_id;
            staged.orphan_progress_notes.push(note);
        }

        // Week notes -----------------------------------------------------------
        for note in other_notes {
            if let Some(index) = staged
                .week_notes
                .iter()
                .position(|existing| existing.week_start == note.week_start)
            {
                if staged.week_notes[index] == note {
                    continue;
                }
                if is_empty_week_note(&staged.week_notes[index]) {
                    staged.week_notes[index] = note;
                    let canonical = staged.week_notes[index].clone();
                    staged
                        .archived_week_notes
                        .retain(|archived| archived != &canonical);
                } else {
                    archive_week_note(&mut staged.archived_week_notes, note);
                }
            } else {
                staged.week_notes.push(note);
            }
        }

        for note in other_archived_notes {
            if staged.week_notes.iter().any(|canonical| canonical == &note) {
                continue;
            }
            archive_week_note(&mut staged.archived_week_notes, note);
        }

        if staged
            .week_notes
            .iter()
            .chain(&staged.archived_week_notes)
            .any(|note| note.markdown.is_some())
        {
            staged.schema_version = staged.schema_version.max(2);
        }
        staged.validate()?;
        let added_count = staged.tasks.len() - existing_tasks.len();
        *self = staged;
        Ok(added_count)
    }

    /// Validates schema version, unique ids, parent references, tag references,
    /// cycles, maximum depth, week-note uniqueness, Markdown attachments and settings.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version >= 1 && self.schema_version <= SCHEMA_VERSION,
            "unsupported schema version {} (this build supports 1..={SCHEMA_VERSION})",
            self.schema_version
        );

        let mut task_by_id: HashMap<&str, &Task> = HashMap::new();
        for task in &self.tasks {
            ensure!(!task.id.trim().is_empty(), "task id must not be empty");
            ensure!(
                task_by_id.insert(task.id.as_str(), task).is_none(),
                "duplicate task id {}",
                task.id
            );
        }

        let tag_ids: HashSet<&str> = self.tags.iter().map(|tag| tag.id.as_str()).collect();
        ensure!(
            tag_ids.len() == self.tags.len(),
            "duplicate tag id in document"
        );
        for tag in &self.tags {
            ensure!(!tag.id.trim().is_empty(), "tag id must not be empty");
        }

        for task in &self.tasks {
            for tag_id in &task.tags {
                ensure!(
                    tag_ids.contains(tag_id.as_str()),
                    "dangling tag reference {tag_id} on task {}",
                    task.id
                );
            }

            let mut note_ids = HashSet::new();
            for note in &task.progress_notes {
                ensure!(
                    !note.id.trim().is_empty(),
                    "progress note id on task {} must not be empty",
                    task.id
                );
                ensure!(
                    note_ids.insert(note.id.as_str()),
                    "duplicate progress note id {} on task {}",
                    note.id,
                    task.id
                );
            }

            let mut chain = HashSet::new();
            let mut current = task;
            let mut depth = 1usize;
            loop {
                ensure!(
                    chain.insert(current.id.as_str()),
                    "cycle detected in task hierarchy near {}",
                    current.id
                );
                let Some(parent_id) = current.parent_id.as_deref() else {
                    break;
                };
                if chain.contains(parent_id) {
                    bail!("cycle detected in task hierarchy near {}", current.id);
                }
                let parent = task_by_id.get(parent_id).copied().ok_or_else(|| {
                    anyhow::anyhow!("dangling parent reference {parent_id} on task {}", task.id)
                })?;
                depth += 1;
                ensure!(
                    depth <= MAX_TASK_DEPTH,
                    "task {} exceeds the maximum task depth of {MAX_TASK_DEPTH}",
                    task.id
                );
                current = parent;
            }
        }

        let mut note_weeks = HashSet::new();
        for note in &self.week_notes {
            ensure!(
                note_weeks.insert(note.week_start),
                "duplicate week note for {}",
                note.week_start
            );
        }

        for (kind, notes) in [
            ("week note", &self.week_notes),
            ("archived week note", &self.archived_week_notes),
        ] {
            for note in notes {
                if let Some(markdown) = &note.markdown {
                    ensure!(
                        self.schema_version >= 2,
                        "{kind} for {} contains Markdown, which requires schema version 2 or newer",
                        note.week_start
                    );
                    markdown
                        .validate()
                        .with_context(|| format!("invalid {kind} for {}", note.week_start))?;
                }
            }
        }

        let mut orphan_note_ids = HashSet::new();
        for note in &self.orphan_progress_notes {
            ensure!(
                !note.id.trim().is_empty(),
                "orphan progress note id must not be empty"
            );
            ensure!(
                orphan_note_ids.insert(note.id.as_str()),
                "duplicate orphan progress note id {}",
                note.id
            );
        }

        self.settings.validate()
    }

    // -- internals ---------------------------------------------------------

    fn index_of(&self, id: &str) -> Option<usize> {
        self.tasks.iter().position(|task| task.id == id)
    }

    fn collect_subtree_ids(&self, root: &str) -> Vec<String> {
        let mut result = vec![root.to_string()];
        let mut seen: HashSet<String> = result.iter().cloned().collect();
        let mut cursor = 0;
        while cursor < result.len() {
            let parent_id = result[cursor].clone();
            cursor += 1;
            for task in &self.tasks {
                if task.parent_id.as_deref() == Some(parent_id.as_str())
                    && seen.insert(task.id.clone())
                {
                    result.push(task.id.clone());
                }
            }
        }
        result
    }

    fn set_subtree_week(&mut self, root: &str, week: NaiveDate) {
        for id in self.collect_subtree_ids(root) {
            if let Some(task) = self.task_mut(&id) {
                task.week_start = week;
            }
        }
    }

    fn mark_completed_in_place(&mut self, id: &str) {
        let now = utc_now();
        if let Some(task) = self.task_mut(id)
            && !task.completed
        {
            task.completed = true;
            task.completed_at = Some(now);
        }
    }

    fn next_sort_order(&self, parent: Option<&str>) -> f64 {
        self.tasks
            .iter()
            .filter(|task| task.parent_id.as_deref() == parent)
            .map(|task| task.sort_order)
            .fold(-1.0_f64, f64::max)
            + 1.0
    }

    /// Depth of an existing task: top level = 1. Errors on missing tasks and
    /// cycles.
    fn depth_of(&self, id: &str) -> Result<usize> {
        let mut current = self
            .task(id)
            .ok_or_else(|| anyhow::anyhow!("task {id} not found"))?;
        let mut visiting = HashSet::new();
        let mut depth = 1usize;
        loop {
            ensure!(
                visiting.insert(current.id.as_str()),
                "cycle detected in task hierarchy near {}",
                current.id
            );
            let Some(parent_id) = current.parent_id.as_deref() else {
                return Ok(depth);
            };
            ensure!(
                !visiting.contains(parent_id),
                "cycle detected in task hierarchy near {}",
                current.id
            );
            current = self
                .task(parent_id)
                .ok_or_else(|| anyhow::anyhow!("dangling parent reference {parent_id}"))?;
            depth += 1;
        }
    }

    /// Height of an existing task's subtree: leaf = 1.
    fn subtree_height(&self, id: &str) -> Result<usize> {
        fn visit(doc: &Document, id: &str, visiting: &mut HashSet<String>) -> Result<usize> {
            ensure!(
                visiting.insert(id.to_string()),
                "cycle detected in task hierarchy near {id}"
            );
            let mut max_child_height = 0usize;
            for child in doc
                .tasks
                .iter()
                .filter(|task| task.parent_id.as_deref() == Some(id))
            {
                max_child_height = max_child_height.max(visit(doc, &child.id, visiting)?);
            }
            visiting.remove(id);
            Ok(1 + max_child_height)
        }

        if self.task(id).is_none() {
            bail!("task {id} not found");
        }
        visit(self, id, &mut HashSet::new())
    }

    /// Returns true when `candidate` is `root` or a descendant of `root`.
    fn is_in_subtree(&self, candidate: &str, root: &str) -> Result<bool> {
        if candidate == root {
            return Ok(true);
        }
        let mut current_id = Some(candidate.to_string());
        let mut visited = HashSet::new();
        while let Some(id) = current_id {
            ensure!(
                visited.insert(id.clone()),
                "cycle detected in task hierarchy near {id}"
            );
            if id == root {
                return Ok(true);
            }
            let task = self
                .task(&id)
                .ok_or_else(|| anyhow::anyhow!("task {id} not found"))?;
            current_id = task.parent_id.clone();
        }
        Ok(false)
    }
}

impl Settings {
    fn validate(&self) -> Result<()> {
        ensure!(
            matches!(
                self.theme.as_str(),
                "system" | "light" | "dark" | "ocean" | "forest" | "terminal"
            ),
            "unsupported theme {}",
            self.theme
        );
        if let Some(appearance) = &self.appearance {
            ensure!(
                appearance.hue.is_finite() && (0.0..=360.0).contains(&appearance.hue),
                "appearance.hue must be between 0 and 360 degrees"
            );
            ensure!(
                appearance.saturation.is_finite() && (0.0..=1.0).contains(&appearance.saturation),
                "appearance.saturation must be between 0 and 1"
            );
            ensure!(
                appearance.dark_lightness.is_finite()
                    && (0.0..=0.20).contains(&appearance.dark_lightness),
                "appearance.darkLightness must be between 0 and 0.20"
            );
            ensure!(
                appearance.accent_hex.len() == 7
                    && appearance.accent_hex.starts_with('#')
                    && appearance.accent_hex.as_bytes()[1..]
                        .iter()
                        .all(u8::is_ascii_hexdigit),
                "appearance.accentHex must be an opaque #RRGGBB color"
            );
        }
        ensure!(
            matches!(self.language.as_str(), "zh" | "en"),
            "unsupported language {}",
            self.language
        );
        ensure!(
            self.font_size.is_finite() && self.font_size > 0.0,
            "fontSize must be a positive finite number"
        );
        ensure!(
            self.opacity.is_finite() && (0.0..=1.0).contains(&self.opacity),
            "opacity must be between 0 and 1"
        );
        if let Some(effects) = &self.window_effects {
            ensure!(
                effects.transparency.is_finite() && (0.0..=1.0).contains(&effects.transparency),
                "windowEffects.transparency must be between 0 and 1"
            );
            ensure!(
                effects.blur_radius.is_finite() && (0.0..=64.0).contains(&effects.blur_radius),
                "windowEffects.blurRadius must be between 0 and 64"
            );
        }
        ensure!(
            self.background_opacity.is_finite() && (0.0..=1.0).contains(&self.background_opacity),
            "backgroundOpacity must be between 0 and 1"
        );
        ensure!(
            self.background_blur.is_finite() && self.background_blur >= 0.0,
            "backgroundBlur must be a non-negative finite number"
        );
        ensure!(
            self.background_scrim.is_finite() && (0.0..=1.0).contains(&self.background_scrim),
            "backgroundScrim must be between 0 and 1"
        );
        ensure!(
            matches!(
                self.background_fill.as_str(),
                "fill" | "fit" | "tile" | "stretch"
            ),
            "unsupported backgroundFill {}",
            self.background_fill
        );
        Ok(())
    }
}

#[cfg(test)]
fn local_midnight_utc(date: NaiveDate) -> DateTime<Utc> {
    let naive = date.and_hms_opt(0, 0, 0).expect("midnight is always valid");
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|| naive.and_utc())
}

fn stable_import_id<T: Serialize + ?Sized>(
    kind: &str,
    source_id: &str,
    value: &T,
) -> Result<String> {
    let content =
        serde_json::to_vec(value).context("failed to serialize imported entity for stable id")?;
    let mut high = 0xcbf2_9ce4_8422_2325_u64;
    let mut low = 0x9e37_79b9_7f4a_7c15_u64;
    let bytes = kind
        .bytes()
        .chain([0x1f])
        .chain(source_id.bytes())
        .chain([0x1e])
        .chain(content.iter().copied());
    for byte in bytes {
        high ^= u64::from(byte);
        high = high.wrapping_mul(0x0000_0100_0000_01b3);
        low = low.rotate_left(5) ^ u64::from(byte);
        low = low.wrapping_mul(0x9e37_79b1_85eb_ca87);
    }
    let hex = format!("{high:016x}{low:016x}");
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

fn is_empty_week_note(note: &WeekNote) -> bool {
    note.text.is_empty() && note.rich_content.is_none() && note.markdown.is_none()
}

fn archive_week_note(archive: &mut Vec<WeekNote>, note: WeekNote) {
    if !archive.iter().any(|existing| existing == &note) {
        archive.push(note);
    }
}

fn compare_tasks(a: &Task, b: &Task) -> Ordering {
    a.sort_order
        .total_cmp(&b.sort_order)
        .then_with(|| a.id.cmp(&b.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn doc_with_task(title: &str) -> (Document, String) {
        let mut doc = Document::default();
        let id = doc.add_task(title, date(2026, 7, 6), 0);
        (doc, id)
    }

    fn link_child(doc: &mut Document, parent: &str, title: &str) -> String {
        let id = doc.add_task(title, date(2026, 7, 6), 0);
        doc.reparent(&id, Some(parent)).unwrap();
        id
    }

    #[test]
    fn week_starts_on_monday() {
        assert_eq!(week_start(date(2026, 7, 6)), date(2026, 7, 6)); // Monday
        assert_eq!(week_start(date(2026, 7, 8)), date(2026, 7, 6));
        assert_eq!(week_start(date(2026, 7, 12)), date(2026, 7, 6)); // Sunday
        assert_eq!(week_start(date(2026, 7, 13)), date(2026, 7, 13));
        assert_eq!(week_start(date(2026, 1, 1)), date(2025, 12, 29));
    }

    #[test]
    fn quadrant_and_agenda_date_follow_original() {
        let mut task = Task {
            urgent: true,
            important: true,
            ..Default::default()
        };
        assert_eq!(task.quadrant(), 0);
        task.important = false;
        assert_eq!(task.quadrant(), 2);
        task.urgent = false;
        task.important = true;
        assert_eq!(task.quadrant(), 1);
        task.important = false;
        assert_eq!(task.quadrant(), 3);

        task.due_date = Some(local_midnight_utc(date(2026, 7, 8)));
        assert_eq!(task.agenda_date(), Some(date(2026, 7, 8)));

        task.completed = true;
        assert_eq!(task.agenda_date(), Some(date(2026, 7, 8)));
        let completed_at = local_midnight_utc(date(2026, 7, 9));
        task.completed_at = Some(completed_at);
        assert_eq!(task.agenda_date(), Some(date(2026, 7, 9)));

        task.completed_at = None;
        assert_eq!(task.agenda_date(), Some(date(2026, 7, 8)));
    }

    #[test]
    fn serde_defaults_and_camel_case_roundtrip() {
        let mut parsed: Document = serde_json::from_value(json!({
            "settings": {},
            "tasks": [{"title": "partial"}],
            "weekNotes": [{"weekStart": "2026-07-06"}]
        }))
        .unwrap();
        assert_eq!(parsed.schema_version, 1);
        assert_eq!(parsed.tasks.len(), 1);
        assert_eq!(parsed.tasks[0].title, "partial");
        assert!(!parsed.tasks[0].id.is_empty());
        assert!(parsed.tasks[0].progress_notes.is_empty());
        assert!(parsed.settings.theme == "system");
        assert!(parsed.settings.language == "zh");
        assert_eq!(parsed.settings.background_fill, "fill");
        assert_eq!(parsed.settings.background_scrim, 0.35);
        assert!(parsed.settings.parse_rules.is_empty());
        assert_eq!(parsed.week_notes[0].week_start, date(2026, 7, 6));
        assert!(parsed.archived_week_notes.is_empty());
        assert!(parsed.orphan_progress_notes.is_empty());

        parsed.tasks[0].rich_content = Some("cnRmZA==".to_string());
        parsed.settings.theme = "ocean".to_string();
        parsed.settings.parse_rules.push(ParseRule {
            name: "rule".to_string(),
            pattern: "(?P<title>.+)".to_string(),
            title_group: 1,
            ..ParseRule::default()
        });
        parsed.tasks[0].progress_notes.push(ProgressNote::default());
        parsed.archived_week_notes.push(WeekNote {
            week_start: date(2026, 7, 6),
            text: "conflict".to_string(),
            rich_content: Some("Y29uZmxpY3Q=".to_string()),
            markdown: None,
        });
        parsed.orphan_progress_notes.push(ProgressNote::default());

        let value = serde_json::to_value(&parsed).unwrap();
        assert!(value.get("schemaVersion").is_some());
        assert!(value.get("weekNotes").is_some());
        assert!(value.get("archivedWeekNotes").is_some());
        assert!(value.get("orphanProgressNotes").is_some());
        assert!(value["tasks"][0].get("progressNotes").is_some());
        assert!(value["tasks"][0].get("richContent").is_some());
        assert!(value["settings"].get("builtinParseTagName").is_some());
        assert!(value["settings"].get("backgroundFill").is_some());
        assert!(value["settings"].get("backgroundScrim").is_some());
        assert!(
            value["settings"]["parseRules"][0]
                .get("issueGroup")
                .is_some()
        );
        assert!(
            value["settings"]["parseRules"][0]
                .get("linkGroup")
                .is_some()
        );

        let roundtrip: Document = serde_json::from_value(value).unwrap();
        assert_eq!(roundtrip, parsed);
    }

    #[test]
    fn completion_preferences_preserve_defaults_and_explicit_disabled_values() {
        for value in [json!({}), json!({"settings": {}})] {
            let doc: Document = serde_json::from_value(value).unwrap();
            assert!(doc.settings.task_confetti);
            assert!(doc.settings.task_completion_sound);
        }
        for (value, confetti, sound) in [
            (json!({"taskConfetti": false}), false, true),
            (json!({"taskCompletionSound": false}), true, false),
            (
                json!({"taskConfetti": false, "taskCompletionSound": false}),
                false,
                false,
            ),
            (
                json!({"taskConfetti": true, "taskCompletionSound": true}),
                true,
                true,
            ),
        ] {
            let settings: Settings = serde_json::from_value(value).unwrap();
            assert_eq!(settings.task_confetti, confetti);
            assert_eq!(settings.task_completion_sound, sound);
            let serialized = serde_json::to_value(&settings).unwrap();
            assert_eq!(serialized["taskConfetti"], confetti);
            assert_eq!(serialized["taskCompletionSound"], sound);
            assert_eq!(
                serde_json::from_value::<Settings>(serialized).unwrap(),
                settings
            );
        }
    }

    fn note_attachment(bytes: &[u8]) -> NoteAttachment {
        NoteAttachment {
            id: "12345678-abcd-4321-9876-123456789abc".to_string(),
            name: "original.png".to_string(),
            mime_type: "image/png".to_string(),
            data: BASE64.encode(bytes),
        }
    }

    #[test]
    fn markdown_is_separate_from_verbatim_legacy_content() {
        let mut note: WeekNote = serde_json::from_value(json!({
            "weekStart": "2026-07-06",
            "text": "  Original\r\n原文\t\u{fffc}  ",
            "richContent": "  bGVnYWN5\r\n"
        }))
        .unwrap();
        let original = note.clone();
        assert!(note.markdown.is_none());
        assert_eq!(note.editable_text(), original.text);

        note.markdown = Some(MarkdownNote {
            source: "# Edited\n\n![image](attachment:12345678-abcd-4321-9876-123456789abc)".into(),
            attachments: vec![note_attachment(&[0, 255, 1, 13, 10])],
        });
        assert_eq!(note.editable_text(), note.markdown.as_ref().unwrap().source);
        let value = serde_json::to_value(&note).unwrap();
        assert_eq!(value["text"], original.text);
        assert_eq!(value["richContent"], original.rich_content.unwrap());
        assert_eq!(value["markdown"]["attachments"][0]["mimeType"], "image/png");
        assert_eq!(serde_json::from_value::<WeekNote>(value).unwrap(), note);

        note.markdown.as_mut().unwrap().source.clear();
        assert_eq!(note.editable_text(), "");
        assert!(!is_empty_week_note(&note));
        assert_eq!(
            serde_json::from_str::<MarkdownNote>("{}").unwrap(),
            MarkdownNote::default()
        );
        let attachment: NoteAttachment = serde_json::from_str("{}").unwrap();
        assert!(Uuid::parse_str(&attachment.id).is_ok());
        assert_eq!(attachment.mime_type, "application/octet-stream");
        assert!(attachment.data.is_empty());
    }

    #[test]
    fn markdown_requires_schema_two_in_canonical_and_archived_notes() {
        for archived in [false, true] {
            let mut document = Document::default();
            let note = WeekNote {
                week_start: date(2026, 7, 6),
                markdown: Some(MarkdownNote::default()),
                ..WeekNote::default()
            };
            if archived {
                document.archived_week_notes.push(note);
            } else {
                document.week_notes.push(note);
            }
            let error = format!("{:#}", document.validate().unwrap_err());
            assert!(error.contains("requires schema version 2"), "{error}");
            assert!(error.contains("2026-07-06"), "{error}");
            if archived {
                assert!(error.contains("archived week note"), "{error}");
            }
            document.schema_version = 2;
            document.validate().unwrap();
            for unsupported in [0, SCHEMA_VERSION + 1] {
                document.schema_version = unsupported;
                assert!(
                    document
                        .validate()
                        .unwrap_err()
                        .to_string()
                        .contains("unsupported schema")
                );
            }
        }
    }

    #[test]
    fn markdown_attachment_validation_checks_ids_mime_and_data() {
        let good = MarkdownNote {
            source: String::new(),
            attachments: vec![note_attachment(b"original bytes")],
        };
        for mime_type in [
            "image/png",
            "image/jpeg",
            "image/webp",
            "image/gif",
            "image/bmp",
            "image/tiff",
            "application/octet-stream",
        ] {
            let mut note = good.clone();
            note.attachments[0].mime_type = mime_type.into();
            note.validate().unwrap();
        }
        let mut invalid = good.clone();
        invalid.attachments[0].id = "not-a-uuid".into();
        assert!(format!("{:#}", invalid.validate().unwrap_err()).contains("valid UUID"));
        invalid = good.clone();
        let mut duplicate = invalid.attachments[0].clone();
        duplicate.id.make_ascii_uppercase();
        invalid.attachments.push(duplicate);
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .to_string()
                .contains("duplicate attachment id")
        );

        for mime_type in ["image/svg+xml", "image/jpg", "text/plain", ""] {
            invalid = good.clone();
            invalid.attachments[0].mime_type = mime_type.into();
            assert!(
                invalid
                    .validate()
                    .unwrap_err()
                    .to_string()
                    .contains("unsupported MIME type")
            );
        }
        for data in ["not base64!", "YQ=", "YR=="] {
            invalid = good.clone();
            invalid.attachments[0].data = data.into();
            let error = format!("{:#}", invalid.validate().unwrap_err());
            assert!(error.contains("invalid base64"), "{error}");
            assert!(error.contains("original.png"), "{error}");
        }
        invalid = good;
        invalid.attachments[0].data.clear();
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .to_string()
                .contains("nonempty data")
        );

        let document = Document {
            schema_version: 2,
            archived_week_notes: vec![WeekNote {
                week_start: date(2026, 7, 6),
                markdown: Some(invalid),
                ..WeekNote::default()
            }],
            ..Document::default()
        };
        let error = format!("{:#}", document.validate().unwrap_err());
        assert!(error.contains("invalid archived week note for 2026-07-06"));
        assert!(error.contains("nonempty data"));
    }

    #[test]
    fn markdown_attachment_size_limits_use_decoded_original_bytes() {
        let mut note = MarkdownNote {
            attachments: vec![note_attachment(&vec![0; MAX_NOTE_ATTACHMENT_BYTES])],
            ..MarkdownNote::default()
        };
        note.validate().unwrap();
        note.attachments[0].data = BASE64.encode(vec![0; MAX_NOTE_ATTACHMENT_BYTES + 1]);
        assert!(
            note.validate()
                .unwrap_err()
                .to_string()
                .contains("20 MiB attachment limit")
        );
        note.attachments[0].data.push_str("AAAA");
        assert!(
            note.validate()
                .unwrap_err()
                .to_string()
                .contains("20 MiB attachment limit")
        );

        note.attachments[0].data = BASE64.encode(vec![0; MAX_NOTE_ATTACHMENT_BYTES]);
        let mut second = note.attachments[0].clone();
        second.id = new_id();
        note.attachments.push(second);
        let mut third = note_attachment(&vec![
            0;
            MAX_NOTE_ATTACHMENTS_BYTES
                - 2 * MAX_NOTE_ATTACHMENT_BYTES
        ]);
        third.id = new_id();
        note.attachments.push(third);
        note.validate().unwrap();
        note.attachments[2].data = BASE64.encode(vec![
            0;
            MAX_NOTE_ATTACHMENTS_BYTES
                - 2 * MAX_NOTE_ATTACHMENT_BYTES
                + 1
        ]);
        assert!(
            note.validate()
                .unwrap_err()
                .to_string()
                .contains("50 MiB total limit")
        );
    }

    #[test]
    fn markdown_merge_preserves_clears_and_image_only_notes() {
        for markdown in [
            MarkdownNote::default(),
            MarkdownNote {
                attachments: vec![note_attachment(b"image bytes")],
                ..MarkdownNote::default()
            },
        ] {
            let canonical = WeekNote {
                week_start: date(2026, 7, 6),
                markdown: Some(markdown),
                ..WeekNote::default()
            };
            assert!(!is_empty_week_note(&canonical));
            let mut base = Document {
                schema_version: 2,
                week_notes: vec![canonical.clone()],
                ..Document::default()
            };
            let legacy = WeekNote {
                week_start: canonical.week_start,
                text: "imported legacy note".into(),
                rich_content: Some("verbatim original RTFD".into()),
                ..WeekNote::default()
            };
            let imported = Document {
                week_notes: vec![legacy.clone()],
                ..Document::default()
            };
            base.merge(imported.clone()).unwrap();
            assert_eq!(base.week_notes, vec![canonical]);
            assert_eq!(base.archived_week_notes, vec![legacy]);
            let after_first = base.clone();
            base.merge(imported).unwrap();
            assert_eq!(base, after_first);
        }
    }

    #[test]
    fn markdown_merge_archives_different_attachment_bytes_and_is_idempotent() {
        let canonical = WeekNote {
            week_start: date(2026, 7, 6),
            text: "original text".into(),
            rich_content: Some("original RTFD".into()),
            markdown: Some(MarkdownNote {
                source: "![image](attachment:12345678-abcd-4321-9876-123456789abc)".into(),
                attachments: vec![note_attachment(b"first image")],
            }),
        };
        let mut variant = canonical.clone();
        variant.markdown.as_mut().unwrap().attachments[0].data = BASE64.encode(b"second image");
        let mut base = Document {
            schema_version: 2,
            week_notes: vec![canonical.clone()],
            ..Document::default()
        };
        let imported = Document {
            schema_version: 2,
            week_notes: vec![variant.clone()],
            archived_week_notes: vec![canonical.clone(), variant.clone()],
            ..Document::default()
        };
        base.merge(imported.clone()).unwrap();
        assert_eq!(base.week_notes, vec![canonical]);
        assert_eq!(base.archived_week_notes, vec![variant]);
        let after_first = base.clone();
        base.merge(imported).unwrap();
        assert_eq!(base, after_first);
    }

    #[test]
    fn merge_promotes_schema_only_when_markdown_is_present() {
        let mut plain = Document::default();
        plain
            .merge(Document {
                schema_version: 2,
                ..Document::default()
            })
            .unwrap();
        assert_eq!(plain.schema_version, 1);
        for archived in [false, true] {
            let week = date(2026, 7, 6);
            let markdown = WeekNote {
                week_start: week,
                markdown: Some(MarkdownNote {
                    attachments: vec![note_attachment(b"image-only note")],
                    ..MarkdownNote::default()
                }),
                ..WeekNote::default()
            };
            let mut imported = Document {
                schema_version: 2,
                ..Document::default()
            };
            if archived {
                imported.archived_week_notes.push(markdown.clone());
            } else {
                imported.week_notes.push(markdown.clone());
            }
            let mut base = Document {
                week_notes: vec![WeekNote {
                    week_start: week,
                    ..WeekNote::default()
                }],
                ..Document::default()
            };
            base.merge(imported.clone()).unwrap();
            assert_eq!(base.schema_version, 2);
            if archived {
                assert_eq!(base.archived_week_notes, vec![markdown]);
            } else {
                assert_eq!(base.week_notes, vec![markdown]);
            }
            let after_first = base.clone();
            base.merge(imported).unwrap();
            assert_eq!(base, after_first);
        }
    }

    #[test]
    fn add_task_uses_quadrants_and_stable_ids() {
        let mut doc = Document::default();
        let id0 = doc.add_task("q0", date(2026, 7, 8), 0);
        let id1 = doc.add_task("q1", date(2026, 7, 6), 1);
        let id2 = doc.add_task("q2", date(2026, 7, 6), 2);
        let id3 = doc.add_task("q3", date(2026, 7, 6), 3);
        assert_eq!(doc.task(&id0).unwrap().quadrant(), 0);
        assert_eq!(doc.task(&id1).unwrap().quadrant(), 1);
        assert_eq!(doc.task(&id2).unwrap().quadrant(), 2);
        assert_eq!(doc.task(&id3).unwrap().quadrant(), 3);
        assert_eq!(doc.task(&id0).unwrap().week_start, date(2026, 7, 6));
        assert_eq!(doc.task(&id0).unwrap().sort_order, 0.0);
        assert_eq!(doc.task(&id1).unwrap().sort_order, 0.0);
        let extra = doc.add_task("q0 again", date(2026, 7, 6), 0);
        let q0_tasks: Vec<_> = doc
            .tasks
            .iter()
            .filter(|task| task.quadrant() == 0)
            .collect();
        assert_eq!(q0_tasks.len(), 2);
        assert_eq!(doc.task(&extra).unwrap().sort_order, 1.0);
        assert_eq!(doc.task(&id0).unwrap().title, "q0");
    }

    #[test]
    fn add_subtask_copies_parent_defaults_and_appends_to_siblings() {
        for quadrant in 0..4 {
            let mut doc = Document {
                tags: vec![Tag::default(), Tag::default()],
                ..Document::default()
            };
            let parent = doc.add_task("parent", date(2026, 7, 13), quadrant);
            let tags: Vec<String> = if quadrant % 2 == 0 {
                doc.tags.iter().map(|tag| tag.id.clone()).collect()
            } else {
                Vec::new()
            };
            let source = doc.task_mut(&parent).unwrap();
            source.tags = tags.clone();
            source.notes = "parent notes".into();
            source.rich_content = Some("parent rich text".into());
            source.links.push("https://example.com/parent".into());
            source.issue_key = Some("PARENT-1".into());
            source.progress_notes.push(ProgressNote::default());
            source.due_date = Some(local_midnight_utc(date(2026, 7, 15)));
            source.remind_at = source.due_date;
            doc.toggle_complete(&parent);

            let first = doc.add_subtask("first", &parent).unwrap();
            doc.task_mut(&first).unwrap().sort_order = 7.0;
            let child = doc.add_subtask("child", &parent).unwrap();
            let task = doc.task(&child).unwrap();
            assert_ne!(task.id, parent);
            assert_ne!(task.id, first);
            assert_eq!(task.title, "child");
            assert_eq!(task.parent_id.as_deref(), Some(parent.as_str()));
            assert_eq!(task.week_start, date(2026, 7, 13));
            assert_eq!(task.quadrant(), quadrant);
            assert_eq!(task.tags, tags);
            assert_eq!(task.sort_order, 8.0);
            assert_eq!(doc.children(&parent)[1].id, child);
            assert!(!task.completed);
            assert!(task.completed_at.is_none());
            assert!(task.due_date.is_none());
            assert!(task.remind_at.is_none());
            assert!(task.notes.is_empty());
            assert!(task.rich_content.is_none());
            assert!(task.links.is_empty());
            assert!(task.issue_key.is_none());
            assert!(task.progress_notes.is_empty());
            doc.validate().unwrap();
        }
    }

    #[test]
    fn subtask_tags_and_quadrant_are_independent_after_creation() {
        let mut doc = Document {
            tags: vec![Tag::default(), Tag::default()],
            ..Document::default()
        };
        let tags: Vec<String> = doc.tags.iter().map(|tag| tag.id.clone()).collect();
        let week = date(2026, 7, 6);
        let parent = doc.add_task("parent", week, 1);
        doc.task_mut(&parent).unwrap().tags = tags.clone();
        let child = doc.add_subtask("child", &parent).unwrap();
        let grandchild = doc.add_subtask("grandchild", &child).unwrap();

        doc.task_mut(&parent).unwrap().tags.clear();
        doc.move_task(&parent, week, 2);
        for id in [&child, &grandchild] {
            assert_eq!(doc.task(id).unwrap().tags, tags);
            assert_eq!(doc.task(id).unwrap().quadrant(), 1);
        }

        doc.task_mut(&child).unwrap().tags.remove(0);
        doc.move_task(&child, week, 3);
        assert!(doc.task(&parent).unwrap().tags.is_empty());
        assert_eq!(doc.task(&parent).unwrap().quadrant(), 2);
        assert_eq!(doc.task(&grandchild).unwrap().tags, tags);
        assert_eq!(doc.task(&grandchild).unwrap().quadrant(), 1);

        // Later children copy their immediate parent's latest values.
        let later_grandchild = doc.add_subtask("later grandchild", &child).unwrap();
        assert_eq!(
            doc.task(&later_grandchild).unwrap().tags,
            vec![tags[1].clone()]
        );
        assert_eq!(doc.task(&later_grandchild).unwrap().quadrant(), 3);
        let later_child = doc.add_subtask("later child", &parent).unwrap();
        assert!(doc.task(&later_child).unwrap().tags.is_empty());
        assert_eq!(doc.task(&later_child).unwrap().quadrant(), 2);
        doc.validate().unwrap();
    }

    #[test]
    fn add_subtask_rejects_missing_parent_and_excess_depth_without_mutation() {
        let (mut doc, parent) = doc_with_task("parent");
        let child = doc.add_subtask("child", &parent).unwrap();
        let grandchild = doc.add_subtask("grandchild", &child).unwrap();
        let before = doc.clone();

        assert!(doc.add_subtask("missing parent", "missing").is_err());
        assert_eq!(doc, before);
        assert!(doc.add_subtask("too deep", &grandchild).is_err());
        assert_eq!(doc, before);
        doc.validate().unwrap();
    }

    #[test]
    fn reparent_keeps_existing_tags_and_quadrants() {
        let mut doc = Document {
            tags: vec![Tag::default(), Tag::default()],
            ..Document::default()
        };
        let parent = doc.add_task("parent", date(2026, 7, 13), 0);
        doc.task_mut(&parent).unwrap().tags = vec![doc.tags[0].id.clone()];
        let task = doc.add_task("existing task", date(2026, 7, 6), 2);
        let own_tags = vec![doc.tags[1].id.clone()];
        doc.task_mut(&task).unwrap().tags = own_tags.clone();
        let child = doc.add_subtask("child", &task).unwrap();
        doc.move_task(&child, date(2026, 7, 6), 3);
        doc.task_mut(&child).unwrap().tags.clear();

        for new_parent in [Some(parent.as_str()), None] {
            doc.reparent(&task, new_parent).unwrap();
            assert_eq!(doc.task(&task).unwrap().parent_id.as_deref(), new_parent);
            assert_eq!(doc.task(&task).unwrap().tags, own_tags);
            assert_eq!(doc.task(&task).unwrap().quadrant(), 2);
            assert!(doc.task(&child).unwrap().tags.is_empty());
            assert_eq!(doc.task(&child).unwrap().quadrant(), 3);
            for id in [&task, &child] {
                assert_eq!(doc.task(id).unwrap().week_start, date(2026, 7, 13));
            }
        }
        doc.validate().unwrap();
    }

    #[test]
    fn toggle_complete_is_not_recursive() {
        let (mut doc, parent) = doc_with_task("parent");
        let child = link_child(&mut doc, &parent, "child");
        doc.toggle_complete(&parent);
        assert!(doc.task(&parent).unwrap().completed);
        assert!(doc.task(&parent).unwrap().completed_at.is_some());
        assert!(!doc.task(&child).unwrap().completed);
        doc.toggle_complete(&parent);
        assert!(!doc.task(&parent).unwrap().completed);
        assert!(doc.task(&parent).unwrap().completed_at.is_none());
    }

    #[test]
    fn delete_cascades_and_move_carries_subtree() {
        let (mut doc, parent) = doc_with_task("parent");
        let child = link_child(&mut doc, &parent, "child");
        let grandchild = link_child(&mut doc, &child, "grandchild");

        doc.move_task(&parent, date(2026, 7, 13), 2);
        for id in [&parent, &child, &grandchild] {
            assert_eq!(
                doc.task(id).unwrap().week_start,
                date(2026, 7, 13),
                "subtree task {id} should move"
            );
        }
        assert_eq!(doc.task(&parent).unwrap().quadrant(), 2);
        assert_eq!(
            doc.task(&child).unwrap().parent_id.as_deref(),
            Some(parent.as_str())
        );
        assert_eq!(
            doc.task(&grandchild).unwrap().parent_id.as_deref(),
            Some(child.as_str())
        );

        doc.delete_task(&parent);
        assert!(doc.tasks.is_empty());
    }

    #[test]
    fn reparent_enforces_depth_cycle_and_follows_week() {
        let (mut doc, a) = doc_with_task("a");
        let b = link_child(&mut doc, &a, "b");
        let c = link_child(&mut doc, &b, "c");

        assert!(doc.reparent(&a, Some(&a)).is_err());
        assert!(doc.reparent(&a, Some(&c)).is_err());

        let d = doc.add_task("d", date(2026, 7, 13), 1);
        // c is at depth 3, d has no children: depth 4 => rejected.
        assert!(doc.reparent(&d, Some(&c)).is_err());
        assert!(doc.task(&d).unwrap().parent_id.is_none());

        let e = doc.add_task("e", date(2026, 7, 13), 1);
        doc.reparent(&e, Some(&a)).unwrap();
        assert_eq!(doc.task(&e).unwrap().parent_id.as_deref(), Some(a.as_str()));
        assert_eq!(doc.task(&e).unwrap().week_start, date(2026, 7, 6));

        doc.reparent(&e, None).unwrap();
        assert!(doc.task(&e).unwrap().parent_id.is_none());
        doc.validate().unwrap();

        assert!(doc.reparent("missing", None).is_err());
        assert!(doc.reparent(&a, Some("missing")).is_err());
    }

    #[test]
    fn schedule_sets_due_and_moves_subtree() {
        let (mut doc, parent) = doc_with_task("parent");
        let child = link_child(&mut doc, &parent, "child");
        for day in [date(2026, 7, 15), date(2026, 11, 1), date(2027, 1, 4)] {
            doc.schedule(&parent, day);
            assert_eq!(doc.task(&parent).unwrap().week_start, week_start(day));
            assert_eq!(doc.task(&child).unwrap().week_start, week_start(day));
            assert_eq!(doc.task(&parent).unwrap().agenda_date(), Some(day));
            assert_eq!(
                doc.task(&parent)
                    .unwrap()
                    .due_date
                    .unwrap()
                    .with_timezone(&Local)
                    .naive_local(),
                day.and_hms_opt(18, 30, 0).unwrap()
            );
            assert_eq!(doc.task(&child).unwrap().due_date, None);
        }
    }

    #[test]
    fn carry_forward_matches_three_original_actions() {
        let old_week = date(2026, 7, 6);
        let new_week = date(2026, 7, 13);
        let mut doc = Document::default();

        // 1. Leaf: move and clear due/reminder.
        let leaf = doc.add_task("leaf", old_week, 0);
        doc.task_mut(&leaf).unwrap().due_date =
            Some(date(2026, 7, 8).and_hms_opt(0, 0, 0).unwrap().and_utc());
        doc.task_mut(&leaf).unwrap().remind_at =
            Some(date(2026, 7, 8).and_hms_opt(9, 0, 0).unwrap().and_utc());

        // 2. Parent with one unfinished and one finished child.
        let partial = doc.add_task("partial", old_week, 1);
        doc.task_mut(&partial).unwrap().notes = "notes".to_string();
        let finished = link_child(&mut doc, &partial, "finished");
        doc.toggle_complete(&finished);
        let unfinished = link_child(&mut doc, &partial, "unfinished");
        let grandchild = link_child(&mut doc, &unfinished, "grandchild");
        doc.task_mut(&unfinished).unwrap().due_date =
            Some(date(2026, 7, 8).and_hms_opt(0, 0, 0).unwrap().and_utc());
        doc.task_mut(&unfinished).unwrap().remind_at =
            Some(date(2026, 7, 8).and_hms_opt(9, 0, 0).unwrap().and_utc());

        // 3. Parent whose children are all finished.
        let all_done = doc.add_task("all done", old_week, 2);
        let done_child = link_child(&mut doc, &all_done, "done child");
        doc.toggle_complete(&done_child);

        let task_count_before = doc.tasks.len();
        doc.carry_forward(old_week, new_week);
        assert_eq!(doc.tasks.len(), task_count_before + 1);
        doc.validate().unwrap();

        let leaf_task = doc.task(&leaf).unwrap();
        assert_eq!(leaf_task.week_start, new_week);
        assert!(leaf_task.due_date.is_none());
        assert!(leaf_task.remind_at.is_none());

        let partial_task = doc.task(&partial).unwrap();
        assert!(partial_task.completed);
        assert!(partial_task.completed_at.is_some());
        assert_eq!(partial_task.week_start, old_week);
        assert_eq!(
            doc.task(&finished).unwrap().parent_id.as_deref(),
            Some(partial.as_str())
        );
        let copy = doc
            .tasks
            .iter()
            .find(|task| task.title == "partial" && task.id != partial)
            .unwrap();
        let copy_id = copy.id.clone();
        assert_eq!(
            doc.task(&unfinished).unwrap().parent_id.as_deref(),
            Some(copy_id.as_str())
        );
        assert_eq!(doc.task(&unfinished).unwrap().week_start, new_week);
        assert!(doc.task(&unfinished).unwrap().due_date.is_none());
        assert_eq!(doc.task(&grandchild).unwrap().week_start, new_week);
        assert_eq!(copy.notes, "notes");
        assert!(copy.progress_notes.is_empty());

        let all_done_task = doc.task(&all_done).unwrap();
        assert!(all_done_task.completed);
        assert_eq!(all_done_task.week_start, old_week);
        assert_eq!(
            doc.task(&done_child).unwrap().parent_id.as_deref(),
            Some(all_done.as_str())
        );
        assert_eq!(
            doc.tasks
                .iter()
                .filter(|task| task.title == "all done")
                .count(),
            1
        );
    }

    #[test]
    fn carry_selected_processes_only_selected_earlier_weeks() {
        let week_one = date(2026, 6, 22);
        let week_two = date(2026, 6, 29);
        let target = date(2026, 7, 13);
        let mut doc = Document::default();

        let leaf = doc.add_task("selected leaf", week_one, 0);
        doc.task_mut(&leaf).unwrap().due_date = Some(local_midnight_utc(date(2026, 6, 24)));
        doc.task_mut(&leaf).unwrap().remind_at = Some(local_midnight_utc(date(2026, 6, 24)));

        let unselected = doc.add_task("unselected", week_one, 1);

        let partial = doc.add_task("selected partial", week_two, 2);
        let open_child = link_child(&mut doc, &partial, "open child");
        let completed_child = link_child(&mut doc, &partial, "completed child");
        doc.toggle_complete(&completed_child);

        let all_done = doc.add_task("unselected all done", week_two, 3);
        let done_child = link_child(&mut doc, &all_done, "done child");
        doc.toggle_complete(&done_child);
        doc.task_mut(&done_child).unwrap().parent_id = Some(all_done.clone());

        let task_count = doc.tasks.len();
        let selected = vec![
            partial.clone(),
            leaf.clone(),
            "missing".to_string(),
            partial.clone(),
        ];
        doc.carry_selected(&selected, target);
        doc.validate().unwrap();

        assert_eq!(doc.tasks.len(), task_count + 1);
        let leaf_task = doc.task(&leaf).unwrap();
        assert_eq!(leaf_task.week_start, target);
        assert!(leaf_task.due_date.is_none());
        assert!(leaf_task.remind_at.is_none());

        let partial_task = doc.task(&partial).unwrap();
        assert!(partial_task.completed);
        assert_eq!(partial_task.week_start, week_two);
        assert_eq!(
            doc.task(&completed_child).unwrap().parent_id.as_deref(),
            Some(partial.as_str())
        );
        let copy = doc
            .tasks
            .iter()
            .find(|task| task.title == "selected partial" && task.id != partial)
            .unwrap();
        assert!(copy.parent_id.is_none());
        assert_eq!(copy.week_start, target);
        assert_eq!(
            doc.task(&open_child).unwrap().parent_id.as_deref(),
            Some(copy.id.as_str())
        );
        assert_eq!(doc.task(&open_child).unwrap().week_start, target);

        let untouched = doc.task(&unselected).unwrap();
        assert_eq!(untouched.week_start, week_one);
        assert!(!untouched.completed);
        let all_done_task = doc.task(&all_done).unwrap();
        assert_eq!(all_done_task.week_start, week_two);
        assert!(!all_done_task.completed);
        assert_eq!(
            doc.tasks
                .iter()
                .filter(|task| task.title == "selected partial")
                .count(),
            2,
            "duplicate ids must not produce duplicate copies"
        );
    }

    #[test]
    fn carry_forward_is_noop_for_same_or_previous_week() {
        let mut doc = Document::default();
        let id = doc.add_task("keep", date(2026, 7, 6), 0);
        doc.carry_forward(date(2026, 7, 6), date(2026, 7, 6));
        assert_eq!(doc.task(&id).unwrap().week_start, date(2026, 7, 6));
    }

    #[test]
    fn merge_preserves_ids_skips_duplicates_and_remaps_collisions() {
        let mut base = Document::default();
        let base_task = base.add_task("base", date(2026, 7, 6), 0);
        base.tags.push(Tag {
            id: "tag-1".to_string(),
            name: "Local".to_string(),
            color_hex: "#112233".to_string(),
            sort_order: 0.0,
        });
        base.week_notes.push(WeekNote {
            week_start: date(2026, 7, 6),
            text: "local note".to_string(),
            rich_content: Some("bG9jYWw=".to_string()),
            markdown: None,
        });

        let mut imported = Document {
            schema_version: SCHEMA_VERSION,
            ..Default::default()
        };
        imported.tags.push(Tag {
            id: "tag-1".to_string(),
            name: "Imported renamed".to_string(),
            color_hex: "#445566".to_string(),
            sort_order: 1.0,
        });
        imported.tags.push(Tag {
            id: "tag-9".to_string(),
            name: "Unique".to_string(),
            color_hex: "#778899".to_string(),
            sort_order: 2.0,
        });
        imported.tasks.push(Task {
            id: "task-conflict".to_string(),
            title: "collision".to_string(),
            week_start: date(2026, 7, 6),
            ..Task::default()
        });
        imported.tasks.push(Task {
            id: "task-child".to_string(),
            title: "imported child".to_string(),
            parent_id: Some("task-conflict".to_string()),
            tags: vec!["tag-1".to_string(), "tag-9".to_string()],
            week_start: date(2026, 7, 6),
            ..Task::default()
        });
        imported.week_notes.push(WeekNote {
            week_start: date(2026, 7, 6),
            text: "imported note".to_string(),
            rich_content: Some("cnRmZA==".to_string()),
            markdown: None,
        });

        // Add a real collision whose content differs.
        base.tasks.push(Task {
            id: "task-conflict".to_string(),
            title: "local collision".to_string(),
            week_start: date(2026, 7, 6),
            ..Task::default()
        });

        let added = base.merge(imported.clone()).unwrap();
        assert_eq!(added, 2);
        assert_eq!(base.tasks.len(), 4);
        assert!(base.task(&base_task).is_some());
        let child = base
            .tasks
            .iter()
            .find(|task| task.title == "imported child")
            .unwrap();
        assert_ne!(child.id, "task-child");
        assert!(base.task("task-conflict").is_some());
        assert_eq!(base.task("task-conflict").unwrap().title, "local collision");
        let mut all_ids: Vec<&str> = base.tasks.iter().map(|task| task.id.as_str()).collect();
        all_ids.sort_unstable();
        all_ids.dedup();
        assert_eq!(all_ids.len(), base.tasks.len());
        let remapped_parent = base
            .tasks
            .iter()
            .find(|task| task.title == "collision")
            .unwrap();
        assert_ne!(remapped_parent.id, "task-conflict");
        assert_eq!(
            child.parent_id.as_deref(),
            Some(remapped_parent.id.as_str())
        );
        let remapped_tag = base
            .tags
            .iter()
            .find(|tag| tag.name == "Imported renamed")
            .unwrap();
        assert_ne!(remapped_tag.id, "tag-1");
        assert!(child.tags.contains(&remapped_tag.id));
        let unique_tag = base.tags.iter().find(|tag| tag.name == "Unique").unwrap();
        assert_eq!(unique_tag.id, "tag-9");
        assert!(child.tags.contains(&unique_tag.id));
        let canonical = base
            .week_notes
            .iter()
            .find(|note| note.week_start == date(2026, 7, 6))
            .unwrap();
        assert_eq!(canonical.text, "local note");
        assert_eq!(canonical.rich_content.as_deref(), Some("bG9jYWw="));
        assert_eq!(base.archived_week_notes.len(), 1);
        assert_eq!(base.archived_week_notes[0].text, "imported note");
        base.validate().unwrap();

        // Re-importing the same conflicting source is idempotent, including
        // the remapped parent, child, and tag ids.
        let task_count = base.tasks.len();
        let tag_count = base.tags.len();
        assert_eq!(base.merge(imported).unwrap(), 0);
        assert_eq!(base.tasks.len(), task_count);
        assert_eq!(base.tags.len(), tag_count);
        assert_eq!(base.archived_week_notes.len(), 1);
        base.validate().unwrap();

        // A collision-free document is idempotent when imported again.
        let mut clean = Document::default();
        clean.add_task("clean import", date(2026, 7, 20), 0);
        let mut target = Document::default();
        assert_eq!(target.merge(clean.clone()).unwrap(), 1);
        assert_eq!(target.merge(clean).unwrap(), 0);
        assert_eq!(target.tasks.len(), 1);
    }

    #[test]
    fn merge_keeps_orphans_and_archives_conflicting_week_note_rtfd() {
        let mut base = Document::default();
        base.week_notes.push(WeekNote {
            week_start: date(2026, 7, 6),
            text: "shared".to_string(),
            rich_content: Some("bG9jYWw=".to_string()),
            markdown: None,
        });
        base.week_notes.push(WeekNote {
            week_start: date(2026, 7, 13),
            text: "canonical".to_string(),
            rich_content: Some("Y2Fub24=".to_string()),
            markdown: None,
        });
        base.orphan_progress_notes.push(ProgressNote {
            id: "orphan-1".to_string(),
            text: "local orphan".to_string(),
            created_at: local_midnight_utc(date(2026, 7, 6)),
        });

        let mut imported = Document::default();
        imported.week_notes.push(WeekNote {
            week_start: date(2026, 7, 6),
            text: "shared".to_string(),
            rich_content: Some("aW1wb3J0ZWQ=".to_string()),
            markdown: None,
        });
        imported.week_notes.push(WeekNote {
            week_start: date(2026, 7, 13),
            text: "different".to_string(),
            rich_content: Some("ZGlmZmVyZW50".to_string()),
            markdown: None,
        });
        let archived_note = WeekNote {
            week_start: date(2026, 7, 6),
            text: "archived version".to_string(),
            rich_content: Some("YXJjaGl2ZWQ=".to_string()),
            markdown: None,
        };
        imported.archived_week_notes.push(archived_note.clone());
        imported.archived_week_notes.push(archived_note);
        imported.orphan_progress_notes.push(ProgressNote {
            id: "orphan-1".to_string(),
            text: "imported orphan".to_string(),
            created_at: local_midnight_utc(date(2026, 7, 7)),
        });
        imported.orphan_progress_notes.push(ProgressNote {
            id: "orphan-2".to_string(),
            text: "unique orphan".to_string(),
            created_at: local_midnight_utc(date(2026, 7, 8)),
        });

        assert_eq!(base.merge(imported.clone()).unwrap(), 0);
        assert_eq!(base.week_notes.len(), 2);
        let canonical = base
            .week_notes
            .iter()
            .find(|note| note.week_start == date(2026, 7, 6))
            .unwrap();
        assert_eq!(canonical.text, "shared");
        assert_eq!(canonical.rich_content.as_deref(), Some("bG9jYWw="));
        let second = base
            .week_notes
            .iter()
            .find(|note| note.week_start == date(2026, 7, 13))
            .unwrap();
        assert_eq!(second.text, "canonical");
        assert_eq!(second.rich_content.as_deref(), Some("Y2Fub24="));

        assert_eq!(base.archived_week_notes.len(), 3);
        assert!(base.archived_week_notes.iter().any(|note| {
            note.week_start == date(2026, 7, 6)
                && note.text == "shared"
                && note.rich_content.as_deref() == Some("aW1wb3J0ZWQ=")
        }));
        assert!(base.archived_week_notes.iter().any(|note| {
            note.week_start == date(2026, 7, 13)
                && note.text == "different"
                && note.rich_content.as_deref() == Some("ZGlmZmVyZW50")
        }));
        assert_eq!(
            base.archived_week_notes
                .iter()
                .filter(|note| note.text == "archived version")
                .count(),
            1,
            "identical archived versions must be de-duplicated"
        );

        assert_eq!(base.orphan_progress_notes.len(), 3);
        assert_eq!(
            base.orphan_progress_notes
                .iter()
                .filter(|note| note.id == "orphan-1")
                .count(),
            1
        );
        assert!(
            base.orphan_progress_notes
                .iter()
                .any(|note| note.text == "imported orphan" && note.id != "orphan-1")
        );
        assert!(
            base.orphan_progress_notes
                .iter()
                .any(|note| note.id == "orphan-2")
        );
        base.validate().unwrap();

        let task_count = base.tasks.len();
        let orphan_count = base.orphan_progress_notes.len();
        let archived_count = base.archived_week_notes.len();
        assert_eq!(base.merge(imported).unwrap(), 0);
        assert_eq!(base.tasks.len(), task_count);
        assert_eq!(base.orphan_progress_notes.len(), orphan_count);
        assert_eq!(base.archived_week_notes.len(), archived_count);
        base.validate().unwrap();
    }

    #[test]
    fn merge_is_transactional_on_invalid_import() {
        let mut base = Document::default();
        base.add_task("base", date(2026, 7, 6), 0);
        let snapshot = base.clone();

        let mut bad = Document::default();
        bad.tasks.push(Task {
            id: "bad".to_string(),
            parent_id: Some("missing".to_string()),
            ..Task::default()
        });
        let err = base.merge(bad).unwrap_err();
        assert!(format!("{err:#}").contains("dangling parent"));
        assert_eq!(base, snapshot);
    }

    #[test]
    fn reorder_changes_parents_and_prevents_cycles() {
        let (mut doc, parent) = doc_with_task("parent");
        let first = link_child(&mut doc, &parent, "first");
        let second = link_child(&mut doc, &parent, "second");
        let third = link_child(&mut doc, &parent, "third");

        doc.reorder(&third, &first).unwrap();
        let children = doc.children(&parent);
        assert_eq!(
            children
                .iter()
                .map(|task| task.title.as_str())
                .collect::<Vec<_>>(),
            vec!["third", "first", "second"]
        );
        assert_eq!(children[0].sort_order, 0.0);
        assert_eq!(children[1].sort_order, 1.0);
        assert_eq!(children[2].sort_order, 2.0);

        // Move a top-level task into the child group.
        let outsider = doc.add_task("outsider", date(2026, 7, 13), 1);
        doc.reorder(&outsider, &second).unwrap();
        assert_eq!(
            doc.task(&outsider).unwrap().parent_id.as_deref(),
            Some(parent.as_str())
        );
        assert_eq!(doc.task(&outsider).unwrap().week_start, date(2026, 7, 6));
        assert_eq!(
            doc.children(&parent)
                .iter()
                .map(|task| task.title.as_str())
                .collect::<Vec<_>>(),
            vec!["third", "first", "outsider", "second"]
        );

        // Cycle: moving parent below one of its descendants.
        assert!(doc.reorder(&parent, &first).is_err());
        // Depth: a 2-high subtree cannot go below depth-2 `first`.
        let with_child = doc.add_task("with child", date(2026, 7, 6), 0);
        let nested = link_child(&mut doc, &with_child, "nested");
        let child_of_nested = link_child(&mut doc, &nested, "child of nested");
        let _ = child_of_nested;
        assert!(doc.reorder(&with_child, &child_of_nested).is_err());
        doc.validate().unwrap();
    }

    #[test]
    fn reorder_preview_matches_commit_without_mutating_document() {
        let mut doc = Document::default();
        let parent = doc.add_task("parent", date(2026, 7, 6), 0);
        let child = link_child(&mut doc, &parent, "child");
        let leaf = link_child(&mut doc, &child, "leaf");
        let other = doc.add_task("other", date(2026, 7, 13), 1);
        let other_child = link_child(&mut doc, &other, "other child");
        let original = doc.clone();
        for (source, target, allowed) in [
            (other.as_str(), child.as_str(), true),
            (leaf.as_str(), other_child.as_str(), true),
            (parent.as_str(), child.as_str(), false),
            (other.as_str(), leaf.as_str(), false),
            (parent.as_str(), parent.as_str(), false),
            ("missing", parent.as_str(), false),
            (parent.as_str(), "missing", false),
        ] {
            assert_eq!(doc.validate_reorder(source, target).is_ok(), allowed);
            assert_eq!(doc, original);
            let mut committed = doc.clone();
            assert_eq!(committed.reorder(source, target).is_ok(), allowed);
            if allowed {
                committed.validate().unwrap();
            } else {
                assert_eq!(committed, original);
            }
        }
    }

    #[test]
    fn reorder_top_level_adopts_target_quadrant_and_week() {
        let mut doc = Document::default();
        let target = doc.add_task("target", date(2026, 7, 13), 1);
        let dragged = doc.add_task("dragged", date(2026, 7, 6), 3);
        let child = link_child(&mut doc, &dragged, "child");
        doc.reorder(&dragged, &target).unwrap();
        let moved = doc.task(&dragged).unwrap();
        assert!(moved.parent_id.is_none());
        assert_eq!(moved.week_start, date(2026, 7, 13));
        assert_eq!(moved.quadrant(), 1);
        assert_eq!(doc.task(&child).unwrap().week_start, date(2026, 7, 13));
        assert_eq!(
            doc.tasks
                .iter()
                .filter(|task| task.parent_id.is_none() && task.quadrant() == 1)
                .find(|task| task.title == "dragged")
                .unwrap()
                .sort_order,
            0.0
        );
        assert!(doc.reorder(&dragged, &dragged).is_err());
    }

    #[test]
    fn validate_catches_cycles_depth_and_dangling_references() {
        let mut doc = Document::default();
        let a = doc.add_task("a", date(2026, 7, 6), 0);
        let b = link_child(&mut doc, &a, "b");
        let c = link_child(&mut doc, &b, "c");
        doc.validate().unwrap();

        // Depth 4.
        let mut too_deep = doc.clone();
        let d = too_deep.add_task("d", date(2026, 7, 6), 0);
        too_deep.task_mut(&d).unwrap().parent_id = Some(c.clone());
        assert!(format!("{:#}", too_deep.validate().unwrap_err()).contains("maximum task depth"));

        // Cycle.
        let mut cyclic = doc.clone();
        cyclic.task_mut(&a).unwrap().parent_id = Some(c.clone());
        assert!(format!("{:#}", cyclic.validate().unwrap_err()).contains("cycle"));

        // Self-cycle.
        let mut self_cycle = doc.clone();
        self_cycle.task_mut(&a).unwrap().parent_id = Some(a.clone());
        assert!(format!("{:#}", self_cycle.validate().unwrap_err()).contains("cycle"));

        // Dangling parent.
        let mut dangling_parent = doc.clone();
        dangling_parent.task_mut(&b).unwrap().parent_id = Some("nope".to_string());
        assert!(
            format!("{:#}", dangling_parent.validate().unwrap_err()).contains("dangling parent")
        );

        // Dangling tag.
        let mut dangling_tag = doc.clone();
        dangling_tag
            .task_mut(&a)
            .unwrap()
            .tags
            .push("missing-tag".to_string());
        assert!(format!("{:#}", dangling_tag.validate().unwrap_err()).contains("dangling tag"));

        // Duplicate ids.
        let mut duplicate = doc.clone();
        duplicate.tasks.push(duplicate.tasks[0].clone());
        assert!(format!("{:#}", duplicate.validate().unwrap_err()).contains("duplicate task id"));

        // Duplicate week notes.
        let mut duplicate_week = doc.clone();
        duplicate_week.week_notes.push(WeekNote {
            week_start: date(2026, 7, 6),
            ..WeekNote::default()
        });
        duplicate_week.week_notes.push(WeekNote {
            week_start: date(2026, 7, 6),
            ..WeekNote::default()
        });
        assert!(
            format!("{:#}", duplicate_week.validate().unwrap_err()).contains("duplicate week note")
        );

        // Unsupported schema version.
        let mut future = doc.clone();
        future.schema_version = SCHEMA_VERSION + 1;
        assert!(format!("{:#}", future.validate().unwrap_err()).contains("schema version"));
    }

    #[test]
    fn children_are_sorted_by_sort_order_then_id() {
        let mut doc = Document::default();
        let parent = doc.add_task("parent", date(2026, 7, 6), 0);
        let a = link_child(&mut doc, &parent, "a");
        let b = link_child(&mut doc, &parent, "b");
        doc.task_mut(&a).unwrap().sort_order = 5.0;
        doc.task_mut(&b).unwrap().sort_order = 1.0;
        assert_eq!(
            doc.children(&parent)
                .iter()
                .map(|task| task.title.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a"]
        );
        assert!(doc.children("missing").is_empty());
    }

    #[test]
    fn native_swift_export_shape_is_accepted() {
        let raw = r##"{
            "schemaVersion": 1,
            "tasks": [{
                "id": "task-1",
                "title": "Native task",
                "notes": "memo",
                "urgent": true,
                "important": false,
                "completed": false,
                "completedAt": null,
                "dueDate": "2026-07-08T00:00:00.000Z",
                "remindAt": null,
                "createdAt": "2026-07-06T08:30:00.123Z",
                "sortOrder": 2.5,
                "weekStart": "2026-07-06",
                "parentId": null,
                "tags": ["tag-1"],
                "links": ["https://example.test"],
                "issueKey": "ABC-9",
                "richContent": "cnRmZA==",
                "progressNotes": [{
                    "id": "note-1",
                    "text": "progress",
                    "createdAt": "2026-07-07T09:00:00.456Z"
                }]
            }],
            "tags": [{
                "id": "tag-1",
                "name": "Native",
                "colorHex": "#AABBCC",
                "sortOrder": 1.0
            }],
            "weekNotes": [{
                "weekStart": "2026-07-06",
                "text": "week memo",
                "richContent": null
            }],
            "settings": {},
            "orphanProgressNotes": [{
                "id": "orphan-1",
                "text": "kept by native helper",
                "createdAt": "2026-07-07T09:00:00.000Z"
            }]
        }"##;

        let document: Document = serde_json::from_str(raw).unwrap();
        document.validate().unwrap();
        assert_eq!(document.schema_version, 1);
        assert_eq!(document.tasks.len(), 1);
        assert_eq!(document.tasks[0].progress_notes.len(), 1);
        assert_eq!(document.tasks[0].progress_notes[0].id, "note-1");
        assert_eq!(document.orphan_progress_notes.len(), 1);
        assert_eq!(document.orphan_progress_notes[0].id, "orphan-1");
        assert_eq!(
            document.orphan_progress_notes[0].text,
            "kept by native helper"
        );
        assert!(document.archived_week_notes.is_empty());
        assert_eq!(document.settings.theme, "system");
        assert_eq!(document.settings.background_fill, "fill");
        assert_eq!(document.settings.background_scrim, 0.35);
        assert_eq!(document.settings.builtin_parse_tag_name, "");
    }

    #[test]
    fn window_effects_defaults_and_partial_json_remain_compatible() {
        let defaults = WindowEffectsSettings::default();
        assert_eq!(defaults.transparency, 0.40);
        assert_eq!(defaults.blur_radius, 24.0);
        assert!(Settings::default().window_effects.is_none());
        for value in [json!({}), json!({"windowEffects": null})] {
            let settings: Settings = serde_json::from_value(value).unwrap();
            assert!(settings.window_effects.is_none());
            settings.validate().unwrap();
        }
        let settings: Settings = serde_json::from_value(json!({"windowEffects": {}})).unwrap();
        assert_eq!(settings.window_effects, Some(defaults.clone()));
        let partial: Settings = serde_json::from_value(json!({
            "windowEffects": {"transparency": 0.75}
        }))
        .unwrap();
        assert_eq!(
            partial.window_effects,
            Some(WindowEffectsSettings {
                transparency: 0.75,
                ..defaults
            })
        );
    }

    #[test]
    fn window_effects_resolve_legacy_dark_light_and_disabled_glass() {
        for (opacity, dark_transparency, light_transparency) in [
            (-1.0, 0.616, 0.8096),
            (0.0, 0.616, 0.8096),
            (0.5, 0.396, 0.6864),
            (1.0, 0.176, 0.5632),
            (2.0, 0.176, 0.5632),
        ] {
            for (dark, transparency) in [(true, dark_transparency), (false, light_transparency)] {
                let settings = Settings {
                    opacity,
                    ..Default::default()
                };
                let original = settings.clone();
                let effects = settings.window_effects_values(dark);
                assert!((effects.transparency - transparency).abs() < 0.000001);
                assert_eq!(effects.blur_radius, 24.0);
                assert_eq!(settings, original, "resolving must not migrate on read");

                let disabled = Settings {
                    glass: false,
                    ..settings
                };
                assert_eq!(
                    disabled.window_effects_values(dark),
                    WindowEffectsSettings {
                        transparency: 0.0,
                        blur_radius: 0.0,
                    }
                );
            }
        }
    }

    #[test]
    fn window_effects_materialization_preserves_appearance_font_and_background() {
        for theme in ["system", "light", "dark", "ocean", "forest", "terminal"] {
            for appearance in [
                None,
                Some(AppearanceSettings {
                    hue: 135.0,
                    saturation: 0.42,
                    dark_lightness: 0.13,
                    accent_hex: "#C389DB".into(),
                }),
            ] {
                for dark in [false, true] {
                    let mut settings = Settings {
                        theme: theme.into(),
                        appearance: appearance.clone(),
                        font_family: "Helvetica".into(),
                        font_size: 18.0,
                        opacity: 0.75,
                        background_image: Some("wallpaper.png".into()),
                        background_blur: 17.0,
                        ..Default::default()
                    };
                    let mut expected = settings.clone();
                    let legacy = settings.window_effects_values(dark);
                    assert_eq!(settings.customize_window_effects(dark), &legacy);
                    expected.window_effects = Some(legacy);
                    assert_eq!(settings, expected);

                    settings.customize_window_effects(!dark).transparency = 0.25;
                    expected.window_effects.as_mut().unwrap().transparency = 0.25;
                    assert_eq!(settings, expected);
                    settings.validate().unwrap();
                }
            }
        }
    }

    #[test]
    fn window_effects_custom_values_roundtrip_and_override_legacy_settings() {
        let custom = WindowEffectsSettings {
            transparency: 0.625,
            blur_radius: 48.5,
        };
        let mut settings = Settings {
            glass: false,
            opacity: 0.12,
            window_effects: Some(custom.clone()),
            background_blur: 96.0,
            ..Default::default()
        };
        for dark in [false, true] {
            assert_eq!(settings.window_effects_values(dark), custom);
            assert_eq!(settings.customize_window_effects(dark), &custom);
        }
        let mut resolved = settings.window_effects_values(true);
        resolved.blur_radius = 0.0;
        assert_ne!(resolved, settings.window_effects_values(true));
        assert_eq!(settings.window_effects, Some(custom));
        let serialized = serde_json::to_value(&settings).unwrap();
        assert_eq!(
            serialized["windowEffects"],
            json!({"transparency": 0.625, "blurRadius": 48.5})
        );
        assert_eq!(serialized["backgroundBlur"], json!(96.0));
        let restored: Settings = serde_json::from_value(serialized).unwrap();
        assert_eq!(restored, settings);
        restored.validate().unwrap();
    }

    #[test]
    fn window_effects_validation_accepts_bounds_and_rejects_invalid_numbers() {
        for (field, invalid) in [
            (
                "transparency",
                [-0.01, 1.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY],
            ),
            (
                "blurRadius",
                [-0.01, 64.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY],
            ),
        ] {
            for value in invalid {
                let mut doc = Document::default();
                let effects = doc.settings.customize_window_effects(true);
                match field {
                    "transparency" => effects.transparency = value,
                    _ => effects.blur_radius = value,
                }
                let error = doc.validate().unwrap_err().to_string();
                assert!(error.contains(&format!("windowEffects.{field}")), "{error}");
            }
        }
        for transparency in [0.0, 1.0] {
            for blur_radius in [0.0, 64.0] {
                let settings = Settings {
                    window_effects: Some(WindowEffectsSettings {
                        transparency,
                        blur_radius,
                    }),
                    ..Default::default()
                };
                settings.validate().unwrap();
            }
        }
    }

    #[test]
    fn appearance_defaults_and_legacy_json_remain_compatible() {
        let defaults = AppearanceSettings::default();
        assert_eq!(defaults.hue, 220.0);
        assert_eq!(defaults.saturation, 0.10);
        assert_eq!(defaults.dark_lightness, 0.05);
        assert_eq!(defaults.accent_hex, "#6AA9EF");
        for theme in ["system", "light", "dark", "ocean", "forest", "terminal"] {
            let settings: Settings = serde_json::from_value(json!({"theme": theme})).unwrap();
            settings.validate().unwrap();
            assert!(settings.appearance.is_none());
            let named = matches!(theme, "ocean" | "forest" | "terminal");
            assert_eq!(settings.theme_mode(), if named { "dark" } else { theme });
            if !named {
                assert_eq!(settings.appearance_values(), defaults);
            }
        }
        let partial: Settings = serde_json::from_value(json!({
            "appearance": {"hue": 120.0}
        }))
        .unwrap();
        assert_eq!(
            partial.appearance_values(),
            AppearanceSettings {
                hue: 120.0,
                ..defaults
            }
        );
    }

    #[test]
    fn appearance_materializes_legacy_colors_and_preserves_effective_font() {
        for (theme, hue, saturation, lightness, accent) in [
            ("ocean", 212.0, 0.35, 0.076, "#7DB8D9"),
            ("forest", 146.0, 0.20, 0.065, "#A3C1A3"),
            ("terminal", 140.0, 0.23, 0.044, "#8FC9A0"),
        ] {
            let mut settings = Settings {
                theme: theme.into(),
                font_family: "Helvetica".into(),
                ..Default::default()
            };
            let resolved = settings.appearance_values();
            assert_eq!(
                resolved,
                AppearanceSettings {
                    hue,
                    saturation,
                    dark_lightness: lightness,
                    accent_hex: accent.into()
                }
            );
            assert!(
                settings.appearance.is_none(),
                "resolution must not migrate on read"
            );
            assert_eq!(settings.theme, theme);
            assert_eq!(settings.customize_appearance().clone(), resolved);
            assert_eq!(settings.theme, "dark");
            assert_eq!(
                settings.font_family,
                if theme == "terminal" {
                    "Menlo"
                } else {
                    "Helvetica"
                }
            );
            settings.customize_appearance().hue = 45.0;
            settings.customize_appearance().accent_hex = "#e08e8e".into();
            assert_eq!(settings.appearance_values().hue, 45.0);
            assert_eq!(settings.appearance_values().accent_hex, "#e08e8e");
            settings.validate().unwrap();
        }
    }

    #[test]
    fn appearance_custom_values_roundtrip_and_survive_materialization() {
        let custom = AppearanceSettings {
            hue: 270.0,
            saturation: 0.4,
            dark_lightness: 0.18,
            accent_hex: "#DB8ABE".into(),
        };
        for theme in ["system", "light", "dark", "ocean", "forest", "terminal"] {
            let mut settings = Settings {
                theme: theme.into(),
                appearance: Some(custom.clone()),
                ..Default::default()
            };
            assert_eq!(settings.appearance_values(), custom);
            assert_eq!(settings.customize_appearance(), &custom);
            let serialized = serde_json::to_value(&settings).unwrap();
            assert_eq!(
                serialized["appearance"].clone(),
                json!({
                    "hue": 270.0, "saturation": 0.4_f32, "darkLightness": 0.18_f32, "accentHex": "#DB8ABE"
                })
            );
            let restored: Settings = serde_json::from_value(serialized).unwrap();
            assert_eq!(restored, settings);
            restored.validate().unwrap();
        }
    }

    #[test]
    fn appearance_validation_bounds_numbers_and_rejects_malformed_hex() {
        for (field, invalid) in [
            (
                "hue",
                vec![-0.01, 360.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY],
            ),
            (
                "saturation",
                vec![-0.01, 1.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY],
            ),
            (
                "darkLightness",
                vec![-0.01, 0.201, f32::NAN, f32::INFINITY, f32::NEG_INFINITY],
            ),
        ] {
            for value in invalid {
                let mut doc = Document::default();
                let appearance = doc.settings.customize_appearance();
                match field {
                    "hue" => appearance.hue = value,
                    "saturation" => appearance.saturation = value,
                    _ => appearance.dark_lightness = value,
                }
                let error = doc.validate().unwrap_err().to_string();
                assert!(error.contains(field), "{error}");
            }
        }
        for invalid in [
            "",
            "6AA9EF",
            "#FFF",
            "#6AA9EF00",
            "#GG0000",
            " #6AA9EF",
            "#é1234",
            "#12345\n",
        ] {
            let mut settings = Settings::default();
            settings.customize_appearance().accent_hex = invalid.into();
            assert!(
                settings
                    .validate()
                    .unwrap_err()
                    .to_string()
                    .contains("accentHex")
            );
        }
        for hue in [0.0, 360.0] {
            for saturation in [0.0, 1.0] {
                for lightness in [0.0, 0.20] {
                    let settings = Settings {
                        appearance: Some(AppearanceSettings {
                            hue,
                            saturation,
                            dark_lightness: lightness,
                            accent_hex: "#aBcDeF".into(),
                        }),
                        ..Default::default()
                    };
                    settings.validate().unwrap();
                }
            }
        }
    }

    #[test]
    fn settings_accept_terminal_theme_and_all_background_fills() {
        for theme in ["system", "light", "dark", "ocean", "forest", "terminal"] {
            for fill in ["fill", "fit", "tile", "stretch"] {
                let settings = Settings {
                    theme: theme.to_string(),
                    background_fill: fill.to_string(),
                    ..Settings::default()
                };
                settings.validate().unwrap();
            }
        }

        assert_eq!(Settings::default().background_scrim, 0.35);
        assert!(
            Settings {
                background_scrim: 1.5,
                ..Settings::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Settings {
                theme: "unknown".to_string(),
                ..Settings::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Settings {
                background_fill: "cover".to_string(),
                ..Settings::default()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn parse_rules_first_match_and_builtin_fallback() {
        let mut settings = Settings {
            builtin_parse_tag_name: "inbox-tag".to_string(),
            ..Default::default()
        };
        settings.parse_rules.push(ParseRule {
            id: "r1".to_string(),
            name: "bracket".to_string(),
            pattern: r"^\[(.+?)\]\s*(.+?)\s*(https?://\S+)?$".to_string(),
            issue_group: 1,
            title_group: 2,
            link_group: 3,
            tag_name: "rule-tag".to_string(),
        });
        let parsed = settings.parse_input("[ABC-12] Fix login https://example.test/x");
        assert_eq!(parsed.issue_key.as_deref(), Some("ABC-12"));
        assert_eq!(parsed.title, "Fix login");
        assert_eq!(parsed.links, vec!["https://example.test/x"]);
        assert_eq!(parsed.tag_name.as_deref(), Some("rule-tag"));

        // A non-matching/invalid rule falls through to built-in parsing.
        settings.parse_rules.insert(
            0,
            ParseRule {
                pattern: "(".to_string(),
                ..ParseRule::default()
            },
        );
        let builtin = settings.parse_input("ABC-123 Buy milk\nhttps://example.test/a.");
        assert_eq!(builtin.issue_key.as_deref(), Some("ABC-123"));
        assert_eq!(builtin.title, "Buy milk");
        assert_eq!(builtin.links, vec!["https://example.test/a"]);
        assert_eq!(builtin.tag_name.as_deref(), Some("inbox-tag"));

        let no_leading_issue = parse_task_input("see ABC-123 later");
        assert_eq!(no_leading_issue.issue_key, None);
        assert_eq!(no_leading_issue.title, "see ABC-123 later");

        let empty_rule_result = ParseRule {
            pattern: r"^$".to_string(),
            title_group: 0,
            ..ParseRule::default()
        };
        assert!(empty_rule_result.apply("not empty").is_none());
    }
}
