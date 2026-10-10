use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use eframe::egui;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const APP_ID: &str = "Timesheet";
const SETTINGS_FILE: &str = "settings.json";
const DATA_FILE: &str = "timesheet.json";

/// App settings. Always stored in the default app folder so they can be found on startup.
#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub clock_24h: bool,
    pub theme: Theme,
    /// Folder holding the timesheet data; `None` means the default app folder.
    pub data_dir: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            clock_24h: true,
            theme: Theme::System,
            data_dir: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl From<Theme> for egui::ThemePreference {
    fn from(theme: Theme) -> Self {
        match theme {
            Theme::System => Self::System,
            Theme::Light => Self::Light,
            Theme::Dark => Self::Dark,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        read_json(&default_dir().join(SETTINGS_FILE))
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        write_json(&default_dir().join(SETTINGS_FILE), self)
    }

    pub fn data_dir(&self) -> PathBuf {
        self.data_dir.clone().unwrap_or_else(default_dir)
    }
}

/// The per-user app folder, e.g. `~/Library/Application Support/Timesheet` or `%APPDATA%\Timesheet\data`.
pub fn default_dir() -> PathBuf {
    eframe::storage_dir(APP_ID).unwrap_or_else(|| PathBuf::from("."))
}

pub fn data_path(dir: &Path) -> PathBuf {
    dir.join(DATA_FILE)
}

/// Copy of the data file as it was last loaded successfully.
pub fn backup_path(dir: &Path) -> PathBuf {
    dir.join("timesheet.backup.json")
}

/// Returns `Ok(None)` if the file doesn't exist.
/// A file that exists but isn't valid JSON is reported as [`io::ErrorKind::InvalidData`].
pub fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    write_bytes(
        path,
        &serde_json::to_vec_pretty(value).map_err(io::Error::other)?,
    )
}

/// Crash-safe write: the new contents go to a temp file, are flushed to disk, and only then
/// atomically replace the old file. At any moment the file on disk is either the complete
/// old version or the complete new one, even across a power cut.
pub fn write_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = path.with_extension("json.tmp");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    // Without this, a power cut after the rename can leave an empty or zero-filled file.
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)?;
    // Persist the rename itself. Unix only; Windows has no directory handles for this,
    // and NTFS journals the rename. Best effort: not every filesystem supports it.
    #[cfg(unix)]
    let _ = fs::File::open(dir).and_then(|d| d.sync_all());
    Ok(())
}

/// Opens a folder in Finder / Explorer / the default file manager.
pub fn reveal(dir: &Path) {
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(cmd).arg(dir).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("timesheet-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn write_replaces_atomically_and_leaves_no_temp_file() {
        let dir = temp_dir("write");
        let path = data_path(&dir);
        write_json(&path, &vec![1, 2, 3]).unwrap();
        write_json(&path, &vec![4, 5]).unwrap();
        assert_eq!(read_json::<Vec<i32>>(&path).unwrap(), Some(vec![4, 5]));
        assert!(!path.with_extension("json.tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_file_is_none_and_damaged_file_is_invalid_data() {
        let dir = temp_dir("read");
        let path = data_path(&dir);
        assert!(read_json::<Vec<i32>>(&path).unwrap().is_none());
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, b"{\"projects\": [").unwrap(); // truncated
        let err = read_json::<Vec<i32>>(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        fs::remove_dir_all(dir).unwrap();
    }
}
