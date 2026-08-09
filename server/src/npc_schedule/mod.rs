pub mod routes;

use onlinerpg_terrain::io::atomic_write;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;

pub use onlinerpg_shared::schedule::ScheduleEntry;

#[derive(Debug, Serialize, Deserialize)]
pub struct ScheduleFile {
    pub schedule: Vec<ScheduleEntry>,
}

const SCHEDULE_FILENAME: &str = "schedule.json";

pub struct NpcIO {
    base_dir: PathBuf,
}

impl NpcIO {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// List NPC names (subdirectories that contain a schedule.json).
    pub async fn list_npcs(&self) -> std::io::Result<Vec<String>> {
        let mut names = Vec::new();
        let mut entries = fs::read_dir(&self.base_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_dir() {
                let schedule_path = entry.path().join(SCHEDULE_FILENAME);
                if fs::try_exists(&schedule_path).await.unwrap_or(false) {
                    if let Some(name) = entry.file_name().to_str() {
                        names.push(name.to_string());
                    }
                }
            }
        }
        names.sort();
        Ok(names)
    }

    fn validate_name(name: &str) -> std::io::Result<()> {
        if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid NPC name",
            ));
        }
        Ok(())
    }

    /// Read and parse a schedule.json for the given NPC.
    pub async fn read_schedule(&self, name: &str) -> std::io::Result<ScheduleFile> {
        Self::validate_name(name)?;
        let path = self.base_dir.join(name).join(SCHEDULE_FILENAME);
        let content = fs::read_to_string(&path).await?;
        let schedule: ScheduleFile = serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(schedule)
    }

    /// Write schedule data back as JSON.
    pub async fn write_schedule(&self, name: &str, data: &ScheduleFile) -> std::io::Result<()> {
        Self::validate_name(name)?;
        let path = self.base_dir.join(name).join(SCHEDULE_FILENAME);
        let content = serde_json::to_string_pretty(data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        atomic_write(&path, content.as_bytes()).await
    }
}

#[cfg(test)]
mod tests {
    use super::{NpcIO, ScheduleEntry, ScheduleFile};
    use crate::test_util::unique_temp_dir;

    #[tokio::test]
    async fn write_schedule_persists_readable_json() {
        let dir = unique_temp_dir("npc_schedule_write");
        let io = NpcIO::new(dir.clone());
        let schedule = ScheduleFile {
            schedule: vec![ScheduleEntry {
                at: "08:00".into(),
                pos: [1.0, 2.0, 3.0],
                rotation: 90.0,
                floor_level: 1,
                label: Some("counter".into()),
                waypoints: vec![[4.0, 5.0, 6.0]],
                ..Default::default()
            }],
        };

        tokio::fs::create_dir_all(dir.join("shopkeeper"))
            .await
            .unwrap();
        io.write_schedule("shopkeeper", &schedule).await.unwrap();

        let stored = io.read_schedule("shopkeeper").await.unwrap();
        assert_eq!(stored.schedule.len(), 1);
        assert_eq!(stored.schedule[0].at, "08:00");
        assert_eq!(stored.schedule[0].waypoints, vec![[4.0, 5.0, 6.0]]);
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
