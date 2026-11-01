//! Editing targets from the UI (ADR-0035): the configuration file is rewritten in
//! place, so its comments and every other setting survive, then validated by the
//! same parser the daemon starts with.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use anyhow::{Context, bail};
use netwatch_adapter_web::{ConfigView, Settings, TargetSpec};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

use crate::config;
use crate::persist;

const KINDS: [&str; 3] = ["icmp", "dns", "https"];

/// The targets in a configuration file's text, as the settings page edits them.
pub fn specs(text: &str) -> anyhow::Result<Vec<TargetSpec>> {
    let document: DocumentMut = text.parse()?;
    let Some(targets) = document.get("target").and_then(Item::as_array_of_tables) else {
        return Ok(Vec::new());
    };
    targets
        .iter()
        .map(|t| {
            let text = |key: &str| t.get(key).and_then(Item::as_str).map(str::to_owned);
            let kind = KINDS
                .into_iter()
                .find(|k| t.contains_key(k))
                .context("a target has none of icmp, dns, or https")?;
            Ok(TargetSpec {
                id: t
                    .get("id")
                    .and_then(Item::as_integer)
                    .and_then(|id| u32::try_from(id).ok()),
                name: text("name"),
                kind: kind.to_owned(),
                address: text(kind).unwrap_or_default(),
                expect_status: t
                    .get("expect_status")
                    .and_then(Item::as_integer)
                    .and_then(|s| u16::try_from(s).ok()),
                expect_body: text("expect_body"),
            })
        })
        .collect()
}

/// Rewrites `text` so that its targets are exactly `targets`, in that order. Targets
/// keep their tables (and the comments around them) where their id is unchanged.
/// New targets are numbered from `next_id` upwards. The result is validated with
/// [`config::parse`].
pub fn edit_targets(text: &str, targets: &[TargetSpec], next_id: u32) -> anyhow::Result<String> {
    let mut document: DocumentMut = text.parse()?;
    let existing: Vec<Table> = document
        .get("target")
        .and_then(Item::as_array_of_tables)
        .map(|a| a.iter().cloned().collect())
        .unwrap_or_default();
    let mut next = next_id;
    let mut edited = ArrayOfTables::new();
    for spec in targets {
        if !KINDS.contains(&spec.kind.as_str()) {
            bail!("unknown target kind {:?}", spec.kind);
        }
        let (id, mut table) = match spec.id {
            Some(id) => {
                let kept = existing
                    .iter()
                    .find(|t| t.get("id").and_then(Item::as_integer) == Some(i64::from(id)))
                    .cloned()
                    .with_context(|| format!("there is no target {id} to change"))?;
                (id, kept)
            }
            None => {
                let id = next;
                next = next.checked_add(1).context("target ids are exhausted")?;
                (id, Table::new())
            }
        };
        table.insert("id", value(i64::from(id)));
        let mut set = |key: &str, v: Option<Item>| match v {
            Some(v) => {
                table.insert(key, v);
            }
            None => {
                table.remove(key);
            }
        };
        set(
            "name",
            spec.name
                .as_deref()
                .filter(|n| !n.trim().is_empty())
                .map(|n| value(n.trim())),
        );
        for kind in KINDS {
            set(
                kind,
                (spec.kind == kind).then(|| value(spec.address.trim())),
            );
        }
        let https = spec.kind == "https";
        set(
            "expect_status",
            spec.expect_status
                .filter(|_| https)
                .map(|s| value(i64::from(s))),
        );
        set(
            "expect_body",
            spec.expect_body
                .as_deref()
                .filter(|b| https && !b.is_empty())
                .map(value),
        );
        edited.push(table);
    }
    document.insert("target", Item::ArrayOfTables(edited));
    let text = document.to_string();
    config::parse(&text)?;
    Ok(text)
}

/// The daemon's [`Settings`]: edits the file it loaded, then restarts it.
pub struct FileSettings {
    /// The file the configuration came from, if any.
    pub path: Option<PathBuf>,
    pub database: PathBuf,
    pub writes: mpsc::Sender<persist::Write>,
}

impl FileSettings {
    fn path(&self) -> Result<&Path, String> {
        self.path
            .as_deref()
            .ok_or_else(|| "netwatch is running on its built-in defaults, so there is no configuration file to edit. Start it with NETWATCH_CONFIG naming one.".to_owned())
    }

    /// One more than the highest target id in the file or anywhere in history, so
    /// that no id is ever reused (ADR-0009).
    fn next_id(&self, text: &str) -> anyhow::Result<u32> {
        let in_file = specs(text)?.iter().filter_map(|t| t.id).max();
        let in_history = netwatch_adapter_sqlite::Store::open(&self.database)?
            .roles()?
            .into_iter()
            .filter_map(|(role, _)| match role {
                netwatch_shared::Role::Target(id)
                | netwatch_shared::Role::Hop { target: id, .. } => Some(id.get()),
                _ => None,
            })
            .max();
        Ok(in_file.max(in_history).map_or(1, |id| id.saturating_add(1)))
    }
}

impl Settings for FileSettings {
    fn view(&self) -> ConfigView {
        let read = self.path().and_then(|path| {
            std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
        });
        let (targets, reason) = match read.as_deref().map(specs) {
            Ok(Ok(targets)) => (targets, None),
            Ok(Err(error)) => (Vec::new(), Some(error.to_string())),
            Err(reason) => (Vec::new(), Some(reason.clone())),
        };
        let writable = self.path.as_deref().is_some_and(writable);
        ConfigView {
            path: self.path.as_ref().map(|p| p.display().to_string()),
            editable: reason.is_none() && writable,
            reason: reason.or_else(|| {
                (!writable).then(|| "the configuration file isn't writable by netwatch".to_owned())
            }),
            targets,
        }
    }

    fn save(&self, targets: Vec<TargetSpec>) -> Result<(), String> {
        let path = self.path()?;
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let next = self.next_id(&text).map_err(|e| e.to_string())?;
        let edited = edit_targets(&text, &targets, next).map_err(|e| format!("{e:#}"))?;
        write_atomically(path, &edited).map_err(|e| format!("writing {}: {e}", path.display()))?;
        tracing::info!(path = %path.display(), targets = targets.len(), "targets changed; restarting");
        restart_soon(self.writes.clone());
        Ok(())
    }
}

/// Whether the daemon may replace `path`: it writes a temporary file beside it, so
/// it needs the directory as well as the file.
fn writable(path: &Path) -> bool {
    let probe = path.with_extension("toml.writable");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    let temporary = path.with_extension("toml.new");
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, path)
}

/// Restarts the daemon with its new configuration (ADR-0035), after the response
/// that triggered it has been sent: flushes the writer, then re-executes this
/// binary. Ambient capabilities survive `execve`, and every descriptor is
/// close-on-exec, so the new process starts clean.
fn restart_soon(writes: mpsc::Sender<persist::Write>) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let (done, flushed) = tokio::sync::oneshot::channel();
        if writes.send(persist::Write::Flush(done)).is_ok() {
            let _ = flushed.blocking_recv();
        }
        use std::os::unix::process::CommandExt;
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        let error = std::process::Command::new("/proc/self/exe")
            .args(args)
            .exec();
        tracing::error!(%error, "cannot restart; exiting so that systemd restarts netwatch");
        std::process::exit(75);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"# netwatch configuration
listen = "127.0.0.1:8732"
database = "netwatch.db"

# ICMP echo.
[[target]]
id = 1
icmp = "1.1.1.1"

# DNS.
[[target]]
id = 11
name = "Cloudflare DNS"
dns = "system"
"#;

    fn spec(id: Option<u32>, kind: &str, address: &str) -> TargetSpec {
        TargetSpec {
            id,
            name: None,
            kind: kind.to_owned(),
            address: address.to_owned(),
            expect_status: None,
            expect_body: None,
        }
    }

    #[test]
    fn the_file_reads_back_as_specs_with_system_left_as_written() {
        let found = specs(FILE).unwrap();
        assert_eq!(found[0], spec(Some(1), "icmp", "1.1.1.1"));
        assert_eq!(found[1].address, "system");
        assert_eq!(found[1].name.as_deref(), Some("Cloudflare DNS"));
    }

    #[test]
    fn editing_keeps_comments_and_settings_renames_adds_and_removes() {
        let mut dns = specs(FILE).unwrap()[1].clone();
        dns.name = Some("Resolver".to_owned());
        let mut https = spec(None, "https", "https://example.com/");
        https.expect_status = Some(204);
        let edited = edit_targets(FILE, &[dns, https], 30).unwrap();
        assert!(edited.starts_with("# netwatch configuration\nlisten"));
        assert!(edited.contains("# DNS.\n[[target]]\nid = 11"));
        assert!(!edited.contains("1.1.1.1"), "target 1 was removed");
        assert!(edited.contains("name = \"Resolver\""));
        let after = specs(&edited).unwrap();
        assert_eq!(after.len(), 2);
        assert_eq!(after[1].id, Some(30));
        assert_eq!(after[1].expect_status, Some(204));
    }

    #[test]
    fn changing_a_targets_kind_leaves_no_trace_of_the_old_one() {
        let edited = edit_targets(FILE, &[spec(Some(1), "dns", "9.9.9.9")], 30).unwrap();
        let after = specs(&edited).unwrap();
        assert_eq!(after, vec![spec(Some(1), "dns", "9.9.9.9")]);
    }

    #[test]
    fn an_invalid_edit_is_rejected_with_the_parsers_reason() {
        let error = edit_targets(FILE, &[spec(None, "icmp", "not an address")], 30).unwrap_err();
        assert!(format!("{error:#}").contains("invalid"), "{error:#}");
        assert!(edit_targets(FILE, &[spec(None, "https", "https://example.com/")], 30).is_err());
        assert!(edit_targets(FILE, &[spec(Some(99), "icmp", "1.1.1.1")], 30).is_err());
        assert!(edit_targets(FILE, &[spec(None, "ftp", "x")], 30).is_err());
    }
}
