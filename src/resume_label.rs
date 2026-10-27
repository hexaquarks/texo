use std::{
    fs,
    io::{ErrorKind, Write},
    path::Path,
};

use anyhow::{Context, Result, ensure};
use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use texo::theme;

pub const MAX_LABEL_WORDS: usize = 3;
pub const MAX_LABEL_CHARS: usize = 24;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelColor {
    #[default]
    Blue,
    Teal,
    Lavender,
    Gold,
}

impl LabelColor {
    pub const ALL: [Self; 4] = [Self::Blue, Self::Teal, Self::Lavender, Self::Gold];

    pub fn name(self) -> &'static str {
        match self {
            Self::Blue => "Blue",
            Self::Teal => "Teal",
            Self::Lavender => "Purple",
            Self::Gold => "Gold",
        }
    }

    pub fn color(self) -> Color {
        match self {
            Self::Blue => theme::ACCENT,
            Self::Teal => theme::TEAL,
            Self::Lavender => theme::LAVENDER,
            Self::Gold => Color::Rgb(249, 226, 175),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeLabel {
    pub name: String,
    pub color: LabelColor,
}

impl ResumeLabel {
    pub fn new(name: &str, color: LabelColor) -> Result<Self> {
        ensure!(
            !name.chars().any(char::is_control),
            "Labels cannot contain control characters"
        );

        let words: Vec<_> = name.split_whitespace().collect();
        ensure!(
            !words.is_empty() && words.len() <= MAX_LABEL_WORDS,
            "Use 1–{MAX_LABEL_WORDS} words"
        );
        let name = words.join(" ");
        ensure!(
            name.chars().count() <= MAX_LABEL_CHARS,
            "Keep labels within {MAX_LABEL_CHARS} characters"
        );

        Ok(Self { name, color })
    }
}

pub const PRESET_LABELS: [(&str, LabelColor); 8] = [
    ("Software", LabelColor::Blue),
    ("Frontend", LabelColor::Lavender),
    ("Backend", LabelColor::Teal),
    ("Full stack", LabelColor::Blue),
    ("Mobile", LabelColor::Lavender),
    ("Data / AI", LabelColor::Gold),
    ("DevOps", LabelColor::Teal),
    ("Research", LabelColor::Gold),
];

const SAVED_LABELS_FILE: &str = ".texo-labels.toml";

#[derive(Default, Serialize, Deserialize)]
struct SavedLabels {
    labels: Vec<ResumeLabel>,
}

impl SavedLabels {
    fn load(root: &Path) -> Result<Self> {
        let text = match fs::read_to_string(root.join(SAVED_LABELS_FILE)) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error).context("Could not read saved labels"),
        };
        let mut saved: Self = toml_edit::de::from_str(&text).context("Invalid saved labels")?;
        for label in &mut saved.labels {
            *label = ResumeLabel::new(&label.name, label.color)?;
        }
        Ok(saved)
    }
}

pub fn label_choices(
    root: &Path,
    projects: &[crate::resume_project::Project],
) -> Result<Vec<ResumeLabel>> {
    let mut choices: Vec<_> = PRESET_LABELS
        .iter()
        .map(|(name, color)| ResumeLabel {
            name: (*name).into(),
            color: *color,
        })
        .collect();
    let saved = SavedLabels::load(root)?;
    let assigned = projects
        .iter()
        .filter_map(|project| project.manifest.label.clone());
    for label in saved.labels.into_iter().chain(assigned) {
        if !choices.contains(&label) {
            choices.push(label);
        }
    }
    Ok(choices)
}

pub fn remember_labels(root: &Path, labels: &[ResumeLabel]) -> Result<()> {
    // Merge the latest catalog so another dialog's saved choices are retained.
    let mut saved = SavedLabels::load(root)?;
    for label in labels {
        let preset = PRESET_LABELS
            .iter()
            .any(|(name, color)| *name == label.name && *color == label.color);
        if !preset && !saved.labels.contains(label) {
            saved.labels.push(label.clone());
        }
    }
    let mut temporary = tempfile::NamedTempFile::new_in(root).context("Could not save labels")?;
    temporary.write_all(toml_edit::ser::to_string_pretty(&saved)?.as_bytes())?;
    temporary
        .persist(root.join(SAVED_LABELS_FILE))
        .context("Could not save labels")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_names_have_bounded_plain_text_without_losing_unicode() -> Result<()> {
        let label = ResumeLabel::new("  Développeur   李  ", LabelColor::Teal)?;
        assert_eq!(label.name, "Développeur 李");
        assert!(ResumeLabel::new("", LabelColor::Blue).is_err());
        assert!(ResumeLabel::new("one two three four", LabelColor::Blue).is_err());
        assert!(ResumeLabel::new(&"x".repeat(MAX_LABEL_CHARS + 1), LabelColor::Blue).is_err());
        assert!(ResumeLabel::new("Role\nName", LabelColor::Blue).is_err());
        assert!(ResumeLabel::new("\u{1b}[31mRed", LabelColor::Blue).is_err());

        for (name, color) in PRESET_LABELS {
            assert_eq!(ResumeLabel::new(name, color)?.name, name);
        }
        Ok(())
    }
}
