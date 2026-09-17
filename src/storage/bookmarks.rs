use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub path: PathBuf,
    pub filename: String,
    /// The leading/left page of the bookmarked spread — same convention as
    /// `storage::history::HistoryEntry::last_page`.
    pub page: usize,
    pub total_pages: usize,
    pub created_secs: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Bookmarks {
    /// Most-recently-added first.
    pub entries: Vec<Bookmark>,
}

impl Bookmarks {
    pub fn bookmarks_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Impossible de trouver le dossier config"))?;
        Ok(config_dir.join("comic-reader").join("bookmarks.json"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::bookmarks_path()?;
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::bookmarks_path()?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        let content = serde_json::to_string_pretty(&self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Whether `path` has a bookmark at exactly `page`.
    pub fn contains(&self, path: &Path, page: usize) -> bool {
        self.entries.iter().any(|b| b.path == path && b.page == page)
    }

    /// Adds a bookmark for `path` at `page`, or removes it if one's already
    /// there — the header's bookmark toggle button. Returns whether `path`
    /// is now bookmarked at `page` (i.e. `true` for added, `false` for
    /// removed).
    pub fn toggle(&mut self, path: &Path, filename: &str, page: usize, total_pages: usize) -> bool {
        if let Some(idx) = self.entries.iter().position(|b| b.path == path && b.page == page) {
            self.entries.remove(idx);
            false
        } else {
            self.entries.insert(0, Bookmark {
                path: path.to_path_buf(),
                filename: filename.to_string(),
                page,
                total_pages,
                created_secs: now_secs(),
            });
            true
        }
    }

    /// Removes the bookmark at exactly `path`/`page`, if any — the
    /// bookmarks panel's per-row remove button.
    pub fn remove_at(&mut self, path: &Path, page: usize) {
        self.entries.retain(|b| !(b.path == path && b.page == page));
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_adds_then_removes() {
        let mut bookmarks = Bookmarks::default();
        let path = PathBuf::from("/tmp/book.cbz");

        assert!(bookmarks.toggle(&path, "book.cbz", 5, 100));
        assert!(bookmarks.contains(&path, 5));
        assert_eq!(bookmarks.entries.len(), 1);

        assert!(!bookmarks.toggle(&path, "book.cbz", 5, 100));
        assert!(!bookmarks.contains(&path, 5));
        assert!(bookmarks.entries.is_empty());
    }

    #[test]
    fn different_pages_of_the_same_book_are_independent() {
        let mut bookmarks = Bookmarks::default();
        let path = PathBuf::from("/tmp/book.cbz");

        bookmarks.toggle(&path, "book.cbz", 5, 100);
        bookmarks.toggle(&path, "book.cbz", 40, 100);
        assert!(bookmarks.contains(&path, 5));
        assert!(bookmarks.contains(&path, 40));
        assert_eq!(bookmarks.entries.len(), 2);

        bookmarks.remove_at(&path, 5);
        assert!(!bookmarks.contains(&path, 5));
        assert!(bookmarks.contains(&path, 40));
    }

    #[test]
    fn remove_at_a_page_never_bookmarked_is_a_no_op() {
        let mut bookmarks = Bookmarks::default();
        let path = PathBuf::from("/tmp/book.cbz");
        bookmarks.remove_at(&path, 5);
        assert!(bookmarks.entries.is_empty());
    }
}
