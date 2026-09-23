use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Most entries a `History` keeps — old books fall off the end rather than
/// growing the file forever.
const MAX_ENTRIES: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub path: PathBuf,
    pub filename: String,
    /// `current_page` at the time this was saved — the leading page of the
    /// spread being shown.
    pub last_page: usize,
    pub total_pages: usize,
    pub last_opened_secs: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    /// Most-recently-opened first.
    pub entries: Vec<HistoryEntry>,
}

impl History {
    pub fn history_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Impossible de trouver le dossier config"))?;
        Ok(config_dir.join("comic-reader").join("history.json"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::history_path()?;
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::history_path()?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        let content = serde_json::to_string_pretty(&self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Moves `path` to the front of the list (inserting it if new) and
    /// records its current progress. Call whenever a book finishes loading
    /// or its reading position changes.
    pub fn touch(&mut self, path: &Path, filename: &str, last_page: usize, total_pages: usize) {
        self.entries.retain(|entry| entry.path != path);
        self.entries.insert(0, HistoryEntry {
            path: path.to_path_buf(),
            filename: filename.to_string(),
            last_page,
            total_pages,
            last_opened_secs: now_secs(),
        });
        self.entries.truncate(MAX_ENTRIES);
    }

    /// Updates the saved page for an already-present entry without moving it
    /// or bumping its timestamp — used for the frequent in-book navigation
    /// saves, as opposed to `touch`'s "just opened this" semantics.
    pub fn update_page(&mut self, path: &Path, last_page: usize) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.path == path) {
            entry.last_page = last_page;
        }
    }

    pub fn remove(&mut self, path: &Path) {
        self.entries.retain(|entry| entry.path != path);
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
