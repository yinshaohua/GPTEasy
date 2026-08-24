use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
const ROTATED_FILES: usize = 5;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IssueLogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueLogRecord {
    pub timestamp_epoch_seconds: i64,
    pub level: IssueLogLevel,
    pub event: String,
    pub message: String,
    pub details: Option<String>,
}

#[derive(Debug)]
pub struct IssueLogStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl IssueLogStore {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            path: root.as_ref().join("issue-log.jsonl"),
            lock: Mutex::new(()),
        }
    }

    pub fn append(
        &self,
        level: IssueLogLevel,
        event: impl Into<String>,
        message: impl Into<String>,
        details: Option<String>,
    ) {
        let Ok(_guard) = self.lock.lock() else {
            return;
        };
        let _ = fs::create_dir_all(self.path.parent().unwrap_or_else(|| Path::new(".")));
        self.rotate_if_needed();
        let record = IssueLogRecord {
            timestamp_epoch_seconds: now_epoch_seconds(),
            level,
            event: sanitize(&event.into()),
            message: sanitize(&message.into()),
            details: details.map(|value| sanitize(&value)),
        };
        if let Ok(line) = serde_json::to_string(&record) {
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
            {
                let _ = writeln!(file, "{line}");
            }
        }
    }

    pub fn list(
        &self,
        since_epoch_seconds: i64,
        level: Option<IssueLogLevel>,
        query: Option<&str>,
    ) -> Vec<IssueLogRecord> {
        let Ok(_guard) = self.lock.lock() else {
            return Vec::new();
        };
        let needle = query
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_lowercase);
        self.read_paths(false)
            .into_iter()
            .filter_map(|line| serde_json::from_str::<IssueLogRecord>(&line).ok())
            .filter(|record| record.timestamp_epoch_seconds >= since_epoch_seconds)
            .filter(|record| level.is_none_or(|expected| record.level == expected))
            .filter(|record| {
                needle.as_ref().is_none_or(|needle| {
                    format!("{} {} {:?}", record.event, record.message, record.details)
                        .to_lowercase()
                        .contains(needle)
                })
            })
            .collect()
    }

    pub fn list_all(
        &self,
        since_epoch_seconds: i64,
        level: Option<IssueLogLevel>,
        query: Option<&str>,
    ) -> Vec<IssueLogRecord> {
        let Ok(_guard) = self.lock.lock() else {
            return Vec::new();
        };
        let needle = query
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_lowercase);
        self.read_paths(true)
            .into_iter()
            .filter_map(|line| serde_json::from_str::<IssueLogRecord>(&line).ok())
            .filter(|record| record.timestamp_epoch_seconds >= since_epoch_seconds)
            .filter(|record| level.is_none_or(|expected| record.level == expected))
            .filter(|record| {
                needle.as_ref().is_none_or(|needle| {
                    format!("{} {} {:?}", record.event, record.message, record.details)
                        .to_lowercase()
                        .contains(needle)
                })
            })
            .collect()
    }

    pub fn format(records: &[IssueLogRecord]) -> String {
        records
            .iter()
            .filter_map(|record| serde_json::to_string(record).ok())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn rotate_if_needed(&self) {
        let Ok(metadata) = fs::metadata(&self.path) else {
            return;
        };
        if metadata.len() < MAX_FILE_BYTES {
            return;
        }
        let oldest = self.path.with_extension(format!("jsonl.{ROTATED_FILES}"));
        let _ = fs::remove_file(oldest);
        for index in (1..ROTATED_FILES).rev() {
            let from = self.path.with_extension(format!("jsonl.{index}"));
            let to = self.path.with_extension(format!("jsonl.{}", index + 1));
            let _ = fs::rename(from, to);
        }
        let first = self.path.with_extension("jsonl.1");
        let _ = fs::rename(&self.path, first);
    }

    fn read_paths(&self, include_rotated: bool) -> Vec<String> {
        let mut paths = vec![self.path.clone()];
        if include_rotated {
            paths.extend(
                (1..=ROTATED_FILES)
                    .rev()
                    .map(|index| self.path.with_extension(format!("jsonl.{index}"))),
            );
        }
        paths
            .into_iter()
            .flat_map(|path| {
                OpenOptions::new()
                    .read(true)
                    .open(path)
                    .ok()
                    .into_iter()
                    .flat_map(|file| BufReader::new(file).lines().filter_map(Result::ok))
            })
            .collect()
    }
}

fn sanitize(value: &str) -> String {
    let mut output = value.to_owned();
    for key in [
        "api_key",
        "apikey",
        "authorization",
        "token",
        "password",
        "secret",
    ] {
        let mut start = 0;
        while let Some(relative) = output[start..].to_lowercase().find(key) {
            let index = start + relative;
            let tail = &output[index..];
            let Some(separator) = tail.find(['=', ':']) else {
                break;
            };
            let value_start = index + separator + 1;
            let value_end = output[value_start..]
                .find([',', ' ', '\n', '\r', '}', '"'])
                .map(|offset| value_start + offset)
                .unwrap_or(output.len());
            output.replace_range(value_start..value_end, "[REDACTED]");
            start = value_start + 10;
        }
    }
    output
}

fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{IssueLogLevel, IssueLogStore};
    use tempfile::tempdir;

    #[test]
    fn records_are_filtered_and_secrets_are_redacted() {
        let directory = tempdir().unwrap();
        let store = IssueLogStore::new(directory.path());
        store.append(
            IssueLogLevel::Error,
            "provider.apply",
            "failed",
            Some("api_key=super-secret".to_owned()),
        );
        let records = store.list(0, Some(IssueLogLevel::Error), Some("provider"));
        assert_eq!(records.len(), 1);
        assert!(
            !records[0]
                .details
                .as_deref()
                .unwrap()
                .contains("super-secret")
        );
    }
}
