//! Local UI continuity. Restoring a session never starts playback or hardware.
use crate::api::{Backend, Intent, Screen};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub dark: bool,
    pub screen: String,
    pub dataset: Option<String>,
    pub episode: Option<u64>,
    pub time: f64,
}

impl Session {
    pub fn snapshot(backend: &dyn Backend, dark: bool) -> Self {
        Self {
            dark,
            screen: backend.screen().label().into(),
            dataset: backend.dataset_id().map(str::to_owned),
            episode: backend.playback().selected,
            time: backend.playback().current_time,
        }
    }

    pub fn restore(&self, backend: &mut dyn Backend, restore_dataset: bool) {
        if restore_dataset {
            if let Some(id) = &self.dataset {
                if backend.library().datasets.iter().any(|d| &d.id == id) {
                    backend.dispatch(Intent::OpenDataset(id.clone()));
                }
            }
        }
        // Never apply the old playhead to an explicitly opened, different dataset.
        if backend.dataset_id() == self.dataset.as_deref() && self.dataset.is_some() {
            if let Some(episode) = self.episode {
                if backend
                    .playback()
                    .episodes
                    .iter()
                    .any(|e| e.index == episode)
                {
                    backend.dispatch(Intent::SelectEpisode(episode));
                    if self.time.is_finite() {
                        backend.dispatch(Intent::Seek(self.time));
                    }
                }
            }
        }
        if let Some(screen) = Screen::ALL.into_iter().find(|s| s.label() == self.screen) {
            backend.dispatch(Intent::Navigate(screen));
        }
    }
}

pub fn path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("DOROBOT_STUDIO_STATE_DIR") {
        return Some(PathBuf::from(dir).join("session.json"));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    #[cfg(target_os = "macos")]
    let dir = home.join("Library/Application Support/DoRobot Studio");
    #[cfg(not(target_os = "macos"))]
    let dir = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/state"))
        .join("dorobot-studio");
    Some(dir.join("session.json"))
}

pub fn load(path: &Path) -> Option<Session> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

pub fn save(path: &Path, session: &Session) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(session)?;
    if std::fs::read(path).is_ok_and(|previous| previous == bytes) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Distinct temporary files let separate app instances save without sharing a writer.
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistence_survives_restart_and_ignores_corrupt_state() {
        let root = std::env::temp_dir().join(format!("dorobot-ui-session-{}", std::process::id()));
        let path = root.join("session.json");
        let session = Session {
            dark: true,
            screen: "Play".into(),
            dataset: Some("sample".into()),
            episode: Some(2),
            time: 1.5,
        };
        save(&path, &session).unwrap();
        assert_eq!(load(&path), Some(session));
        std::fs::write(&path, b"interrupted").unwrap();
        assert!(load(&path).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
