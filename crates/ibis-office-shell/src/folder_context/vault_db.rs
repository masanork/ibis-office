// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Persistent Turso (SQLite) cache for folder/vault Markdown metadata, tags, aliases, and links.
//! Stored under the application data directory so the opened folder or Obsidian Vault is never modified.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use turso_core::{
    Connection, Database, Numeric, OpenFlags, OpenOptions, PlatformIO, SqliteDialect, Value,
};

use ibis_formats::form_assist::{
    FormMapping, PresentationRequestNote, VaultCredentialSummary, VaultProfile,
};

use super::markdown_index::{CachedCredential, CachedFile, RawLink, Stamp};

const SCHEMA_VERSION: &str = "3";

const SCHEMA_SQL: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS files (
        path TEXT PRIMARY KEY,
        size INTEGER NOT NULL,
        mtime_secs INTEGER NOT NULL,
        mtime_nanos INTEGER NOT NULL,
        partial INTEGER NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS tags (
        path TEXT NOT NULL,
        tag TEXT NOT NULL,
        PRIMARY KEY (path, tag)
    )",
    "CREATE INDEX IF NOT EXISTS idx_tags_tag ON tags(tag)",
    "CREATE TABLE IF NOT EXISTS aliases (
        path TEXT NOT NULL,
        alias_lower TEXT NOT NULL,
        PRIMARY KEY (path, alias_lower)
    )",
    "CREATE INDEX IF NOT EXISTS idx_aliases_lower ON aliases(alias_lower)",
    "CREATE TABLE IF NOT EXISTS links (
        source_path TEXT NOT NULL,
        target_raw TEXT NOT NULL,
        is_wiki INTEGER NOT NULL,
        heading TEXT,
        line INTEGER NOT NULL,
        excerpt TEXT NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS idx_links_source ON links(source_path)",
    "CREATE TABLE IF NOT EXISTS vault_profiles (
        path TEXT PRIMARY KEY,
        profile_json TEXT NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS frontmatter_props (
        path TEXT NOT NULL,
        doc_type TEXT NOT NULL,
        key_path TEXT NOT NULL,
        value_json TEXT NOT NULL,
        PRIMARY KEY (path, key_path)
    )",
    "CREATE INDEX IF NOT EXISTS idx_fm_type_key ON frontmatter_props(doc_type, key_path)",
    "CREATE TABLE IF NOT EXISTS form_definitions (
        path TEXT PRIMARY KEY,
        kind TEXT NOT NULL,
        form_id TEXT NOT NULL,
        template_sha256 TEXT,
        sheet_hint TEXT,
        spec_json TEXT NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS idx_form_def_sha ON form_definitions(template_sha256)",
    "CREATE TABLE IF NOT EXISTS vault_credentials (
        path TEXT PRIMARY KEY,
        size INTEGER NOT NULL,
        mtime_secs INTEGER NOT NULL,
        mtime_nanos INTEGER NOT NULL,
        summary_json TEXT
    )",
];

fn db_path_for_root(root: &Path) -> Option<PathBuf> {
    let dir = ibis_office_base::app_data::base()?
        .join("ibisOffice")
        .join("vault-index");
    let mut hasher = Sha256::new();
    hasher.update(root.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(32);
    for byte in &digest[..16] {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    Some(dir.join(format!("{hex}.sqlite")))
}

fn remove_db_files(path: &Path) {
    let _ = std::fs::remove_file(path);
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(sidecar));
    }
}

fn open_connection(path: &Path) -> Result<(Arc<Database>, Arc<Connection>), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let path_str = path
        .to_str()
        .ok_or_else(|| "Vault index path is not valid UTF-8".to_string())?;
    let io = Arc::new(PlatformIO::new().map_err(|err| err.to_string())?);
    let options = OpenOptions::new(Arc::new(SqliteDialect)).flags(OpenFlags::Create);
    let database = Database::open(io, path_str, options).map_err(|err| err.to_string())?;
    let connection = database.connect().map_err(|err| err.to_string())?;
    Ok((database, connection))
}

fn bind(stmt: &mut turso_core::Statement, index: usize, value: Value) -> Result<(), String> {
    let idx = NonZeroUsize::new(index).ok_or_else(|| "Bind index must be >= 1".to_string())?;
    stmt.bind_at(idx, value).map_err(|err| err.to_string())
}

fn ensure_schema(connection: &Arc<Connection>) -> Result<bool, String> {
    for sql in SCHEMA_SQL {
        connection.execute(sql).map_err(|err| err.to_string())?;
    }
    let mut stmt = connection
        .prepare("SELECT value FROM meta WHERE key = 'schema_version'")
        .map_err(|err| err.to_string())?;
    let rows = stmt.run_collect_rows().map_err(|err| err.to_string())?;
    match rows.first().map(Vec::as_slice) {
        Some([Value::Text(version)]) if version.as_str() == SCHEMA_VERSION => Ok(true),
        None => {
            let mut insert = connection
                .prepare("INSERT OR REPLACE INTO meta (key, value) VALUES ('schema_version', ?1)")
                .map_err(|err| err.to_string())?;
            bind(&mut insert, 1, Value::from_text(SCHEMA_VERSION.to_owned()))?;
            insert.run_ignore_rows().map_err(|err| err.to_string())?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn open_or_recreate(root: &Path) -> Option<(Arc<Database>, Arc<Connection>)> {
    let path = db_path_for_root(root)?;
    match open_connection(&path) {
        Ok((db, conn)) => match ensure_schema(&conn) {
            Ok(true) => Some((db, conn)),
            _ => {
                drop(conn);
                drop(db);
                remove_db_files(&path);
                let (db, conn) = open_connection(&path).ok()?;
                ensure_schema(&conn).ok().filter(|ok| *ok)?;
                Some((db, conn))
            }
        },
        Err(_) => {
            remove_db_files(&path);
            let (db, conn) = open_connection(&path).ok()?;
            ensure_schema(&conn).ok().filter(|ok| *ok)?;
            Some((db, conn))
        }
    }
}

fn value_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Numeric(Numeric::Integer(n)) => Some(*n),
        _ => None,
    }
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::Text(text) => Some(text.as_str().to_owned()),
        _ => None,
    }
}

fn stamp_to_parts(stamp: Stamp) -> (i64, i64, i64) {
    let duration = stamp
        .modified
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    (
        i64::try_from(stamp.size).unwrap_or(i64::MAX),
        i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
        i64::from(duration.subsec_nanos()),
    )
}

fn stamp_from_parts(size: i64, secs: i64, nanos: i64) -> Option<Stamp> {
    let size = u64::try_from(size).ok()?;
    let secs = u64::try_from(secs).ok()?;
    let nanos = u32::try_from(nanos).ok()?;
    let modified = UNIX_EPOCH.checked_add(Duration::new(secs, nanos))?;
    Some(Stamp { size, modified })
}

pub(super) fn load_all(root: &Path) -> Option<BTreeMap<String, CachedFile>> {
    let (_db, conn) = open_or_recreate(root)?;
    let mut files = BTreeMap::new();

    {
        let mut stmt = conn
            .prepare("SELECT path, size, mtime_secs, mtime_nanos, partial FROM files")
            .ok()?;
        let rows = stmt.run_collect_rows().ok()?;
        for row in rows {
            if row.len() != 5 {
                continue;
            }
            let Some(path) = value_text(&row[0]) else {
                continue;
            };
            let (Some(size), Some(secs), Some(nanos), Some(partial)) = (
                value_i64(&row[1]),
                value_i64(&row[2]),
                value_i64(&row[3]),
                value_i64(&row[4]),
            ) else {
                continue;
            };
            let Some(stamp) = stamp_from_parts(size, secs, nanos) else {
                continue;
            };
            files.insert(
                path,
                CachedFile {
                    stamp,
                    partial: partial != 0,
                    tags: Vec::new(),
                    aliases: Vec::new(),
                    links: Vec::new(),
                    profile: None,
                    form_mapping: None,
                    presentation_request: None,
                },
            );
        }
    }

    {
        let mut stmt = conn.prepare("SELECT path, tag FROM tags").ok()?;
        let rows = stmt.run_collect_rows().ok()?;
        for row in rows {
            if row.len() != 2 {
                continue;
            }
            if let (Some(path), Some(tag)) = (value_text(&row[0]), value_text(&row[1]))
                && let Some(entry) = files.get_mut(&path)
            {
                entry.tags.push(tag);
            }
        }
    }

    {
        let mut stmt = conn.prepare("SELECT path, alias_lower FROM aliases").ok()?;
        let rows = stmt.run_collect_rows().ok()?;
        for row in rows {
            if row.len() != 2 {
                continue;
            }
            if let (Some(path), Some(alias)) = (value_text(&row[0]), value_text(&row[1]))
                && let Some(entry) = files.get_mut(&path)
            {
                entry.aliases.push(alias);
            }
        }
    }

    {
        let mut stmt = conn
            .prepare("SELECT source_path, target_raw, is_wiki, heading, line, excerpt FROM links")
            .ok()?;
        let rows = stmt.run_collect_rows().ok()?;
        for row in rows {
            if row.len() != 6 {
                continue;
            }
            let (Some(source), Some(target_raw), Some(is_wiki), Some(line), Some(excerpt)) = (
                value_text(&row[0]),
                value_text(&row[1]),
                value_i64(&row[2]),
                value_i64(&row[4]),
                value_text(&row[5]),
            ) else {
                continue;
            };
            let heading = match &row[3] {
                Value::Null => None,
                other => value_text(other),
            };
            let Ok(line) = usize::try_from(line) else {
                continue;
            };
            if let Some(entry) = files.get_mut(&source) {
                entry.links.push(RawLink {
                    target_raw,
                    is_wiki: is_wiki != 0,
                    heading,
                    line,
                    excerpt,
                });
            }
        }
    }

    {
        let mut stmt = conn
            .prepare("SELECT path, profile_json FROM vault_profiles")
            .ok()?;
        let rows = stmt.run_collect_rows().ok()?;
        for row in rows {
            if row.len() != 2 {
                continue;
            }
            if let (Some(path), Some(json_str)) = (value_text(&row[0]), value_text(&row[1]))
                && let Some(entry) = files.get_mut(&path)
                && let Ok(profile) = serde_json::from_str::<VaultProfile>(&json_str)
            {
                entry.profile = Some(profile);
            }
        }
    }

    {
        let mut stmt = conn
            .prepare("SELECT path, kind, spec_json FROM form_definitions")
            .ok()?;
        let rows = stmt.run_collect_rows().ok()?;
        for row in rows {
            if row.len() != 3 {
                continue;
            }
            if let (Some(path), Some(kind), Some(json_str)) = (
                value_text(&row[0]),
                value_text(&row[1]),
                value_text(&row[2]),
            ) && let Some(entry) = files.get_mut(&path)
            {
                match kind.as_str() {
                    "form-mapping" => {
                        if let Ok(mapping) = serde_json::from_str::<FormMapping>(&json_str) {
                            entry.form_mapping = Some(mapping);
                        }
                    }
                    "presentation-request" => {
                        if let Ok(req) =
                            serde_json::from_str::<PresentationRequestNote>(&json_str)
                        {
                            entry.presentation_request = Some(req);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Some(files)
}

pub(super) fn load_credentials(root: &Path) -> Option<BTreeMap<String, CachedCredential>> {
    let (_db, conn) = open_or_recreate(root)?;
    let mut creds = BTreeMap::new();
    let mut stmt = conn
        .prepare("SELECT path, size, mtime_secs, mtime_nanos, summary_json FROM vault_credentials")
        .ok()?;
    let rows = stmt.run_collect_rows().ok()?;
    for row in rows {
        if row.len() != 5 {
            continue;
        }
        let (Some(path), Some(size), Some(secs), Some(nanos)) = (
            value_text(&row[0]),
            value_i64(&row[1]),
            value_i64(&row[2]),
            value_i64(&row[3]),
        ) else {
            continue;
        };
        let Some(stamp) = stamp_from_parts(size, secs, nanos) else {
            continue;
        };
        let summary = match &row[4] {
            Value::Null => None,
            other => value_text(other)
                .and_then(|s| serde_json::from_str::<VaultCredentialSummary>(&s).ok()),
        };
        creds.insert(path, CachedCredential { stamp, summary });
    }
    Some(creds)
}

pub(super) fn sync_diff(
    root: &Path,
    removed: &[String],
    updated: &[(String, &CachedFile)],
) -> Result<(), String> {
    if removed.is_empty() && updated.is_empty() {
        return Ok(());
    }
    let Some((_db, conn)) = open_or_recreate(root) else {
        return Ok(());
    };
    conn.execute("BEGIN IMMEDIATE")
        .map_err(|err| err.to_string())?;
    let outcome = (|| -> Result<(), String> {
        let mut del_file = conn
            .prepare("DELETE FROM files WHERE path = ?1")
            .map_err(|err| err.to_string())?;
        let mut del_tags = conn
            .prepare("DELETE FROM tags WHERE path = ?1")
            .map_err(|err| err.to_string())?;
        let mut del_aliases = conn
            .prepare("DELETE FROM aliases WHERE path = ?1")
            .map_err(|err| err.to_string())?;
        let mut del_links = conn
            .prepare("DELETE FROM links WHERE source_path = ?1")
            .map_err(|err| err.to_string())?;
        let mut del_profile = conn
            .prepare("DELETE FROM vault_profiles WHERE path = ?1")
            .map_err(|err| err.to_string())?;
        let mut del_props = conn
            .prepare("DELETE FROM frontmatter_props WHERE path = ?1")
            .map_err(|err| err.to_string())?;
        let mut del_form_def = conn
            .prepare("DELETE FROM form_definitions WHERE path = ?1")
            .map_err(|err| err.to_string())?;

        for path in removed {
            for stmt in [
                &mut del_file,
                &mut del_tags,
                &mut del_aliases,
                &mut del_links,
                &mut del_profile,
                &mut del_props,
                &mut del_form_def,
            ] {
                stmt.reset().map_err(|err| err.to_string())?;
                bind(stmt, 1, Value::from_text(path.clone()))?;
                stmt.run_ignore_rows().map_err(|err| err.to_string())?;
            }
        }

        let mut upsert_file = conn
            .prepare(
                "INSERT OR REPLACE INTO files (path, size, mtime_secs, mtime_nanos, partial)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(|err| err.to_string())?;
        let mut ins_tag = conn
            .prepare("INSERT OR IGNORE INTO tags (path, tag) VALUES (?1, ?2)")
            .map_err(|err| err.to_string())?;
        let mut ins_alias = conn
            .prepare("INSERT OR IGNORE INTO aliases (path, alias_lower) VALUES (?1, ?2)")
            .map_err(|err| err.to_string())?;
        let mut ins_link = conn
            .prepare(
                "INSERT INTO links (source_path, target_raw, is_wiki, heading, line, excerpt)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|err| err.to_string())?;
        let mut ins_profile = conn
            .prepare("INSERT OR REPLACE INTO vault_profiles (path, profile_json) VALUES (?1, ?2)")
            .map_err(|err| err.to_string())?;
        let mut ins_prop = conn
            .prepare(
                "INSERT OR REPLACE INTO frontmatter_props (path, doc_type, key_path, value_json)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|err| err.to_string())?;
        let mut ins_form_def = conn
            .prepare(
                "INSERT OR REPLACE INTO form_definitions (path, kind, form_id, template_sha256, sheet_hint, spec_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|err| err.to_string())?;

        for (path, entry) in updated {
            for stmt in [
                &mut del_tags,
                &mut del_aliases,
                &mut del_links,
                &mut del_profile,
                &mut del_props,
                &mut del_form_def,
            ] {
                stmt.reset().map_err(|err| err.to_string())?;
                bind(stmt, 1, Value::from_text(path.clone()))?;
                stmt.run_ignore_rows().map_err(|err| err.to_string())?;
            }
            let (size, secs, nanos) = stamp_to_parts(entry.stamp);
            upsert_file.reset().map_err(|err| err.to_string())?;
            bind(&mut upsert_file, 1, Value::from_text(path.clone()))?;
            bind(&mut upsert_file, 2, Value::Numeric(Numeric::Integer(size)))?;
            bind(&mut upsert_file, 3, Value::Numeric(Numeric::Integer(secs)))?;
            bind(&mut upsert_file, 4, Value::Numeric(Numeric::Integer(nanos)))?;
            bind(
                &mut upsert_file,
                5,
                Value::Numeric(Numeric::Integer(i64::from(entry.partial))),
            )?;
            upsert_file.run_ignore_rows().map_err(|err| err.to_string())?;

            for tag in &entry.tags {
                ins_tag.reset().map_err(|err| err.to_string())?;
                bind(&mut ins_tag, 1, Value::from_text(path.clone()))?;
                bind(&mut ins_tag, 2, Value::from_text(tag.clone()))?;
                ins_tag.run_ignore_rows().map_err(|err| err.to_string())?;
            }
            for alias in &entry.aliases {
                ins_alias.reset().map_err(|err| err.to_string())?;
                bind(&mut ins_alias, 1, Value::from_text(path.clone()))?;
                bind(&mut ins_alias, 2, Value::from_text(alias.clone()))?;
                ins_alias.run_ignore_rows().map_err(|err| err.to_string())?;
            }
            for link in &entry.links {
                ins_link.reset().map_err(|err| err.to_string())?;
                bind(&mut ins_link, 1, Value::from_text(path.clone()))?;
                bind(&mut ins_link, 2, Value::from_text(link.target_raw.clone()))?;
                bind(
                    &mut ins_link,
                    3,
                    Value::Numeric(Numeric::Integer(i64::from(link.is_wiki))),
                )?;
                bind(
                    &mut ins_link,
                    4,
                    match &link.heading {
                        Some(heading) => Value::from_text(heading.clone()),
                        None => Value::Null,
                    },
                )?;
                bind(
                    &mut ins_link,
                    5,
                    Value::Numeric(Numeric::Integer(
                        i64::try_from(link.line).unwrap_or(i64::MAX),
                    )),
                )?;
                bind(&mut ins_link, 6, Value::from_text(link.excerpt.clone()))?;
                ins_link.run_ignore_rows().map_err(|err| err.to_string())?;
            }
            if let Some(profile) = &entry.profile
                && let Ok(json_str) = serde_json::to_string(profile)
            {
                ins_profile.reset().map_err(|err| err.to_string())?;
                bind(&mut ins_profile, 1, Value::from_text(path.clone()))?;
                bind(&mut ins_profile, 2, Value::from_text(json_str))?;
                ins_profile.run_ignore_rows().map_err(|err| err.to_string())?;

                for (k, v) in &profile.fields {
                    if let Ok(v_json) = serde_json::to_string(v) {
                        ins_prop.reset().map_err(|err| err.to_string())?;
                        bind(&mut ins_prop, 1, Value::from_text(path.clone()))?;
                        bind(&mut ins_prop, 2, Value::from_text("profile".to_string()))?;
                        bind(&mut ins_prop, 3, Value::from_text(k.clone()))?;
                        bind(&mut ins_prop, 4, Value::from_text(v_json))?;
                        ins_prop.run_ignore_rows().map_err(|err| err.to_string())?;
                    }
                }
            }
            if let Some(mapping) = &entry.form_mapping
                && let Ok(json_str) = serde_json::to_string(mapping)
            {
                ins_form_def.reset().map_err(|err| err.to_string())?;
                bind(&mut ins_form_def, 1, Value::from_text(path.clone()))?;
                bind(&mut ins_form_def, 2, Value::from_text("form-mapping".to_string()))?;
                bind(&mut ins_form_def, 3, Value::from_text(mapping.form_id.clone()))?;
                bind(
                    &mut ins_form_def,
                    4,
                    match &mapping.matcher.template_sha256 {
                        Some(sha) => Value::from_text(sha.clone()),
                        None => Value::Null,
                    },
                )?;
                bind(
                    &mut ins_form_def,
                    5,
                    match &mapping.matcher.sheet {
                        Some(sheet) => Value::from_text(sheet.clone()),
                        None => Value::Null,
                    },
                )?;
                bind(&mut ins_form_def, 6, Value::from_text(json_str))?;
                ins_form_def.run_ignore_rows().map_err(|err| err.to_string())?;
            }
            if let Some(req) = &entry.presentation_request
                && let Ok(json_str) = serde_json::to_string(req)
            {
                ins_form_def.reset().map_err(|err| err.to_string())?;
                bind(&mut ins_form_def, 1, Value::from_text(path.clone()))?;
                bind(
                    &mut ins_form_def,
                    2,
                    Value::from_text("presentation-request".to_string()),
                )?;
                bind(&mut ins_form_def, 3, Value::from_text(req.request_id.clone()))?;
                bind(&mut ins_form_def, 4, Value::Null)?;
                bind(&mut ins_form_def, 5, Value::Null)?;
                bind(&mut ins_form_def, 6, Value::from_text(json_str))?;
                ins_form_def.run_ignore_rows().map_err(|err| err.to_string())?;
            }
        }
        Ok(())
    })();

    match outcome {
        Ok(()) => {
            conn.execute("COMMIT").map_err(|err| err.to_string())?;
            Ok(())
        }
        Err(err) => {
            let _ = conn.execute("ROLLBACK");
            Err(err)
        }
    }
}

pub(super) fn sync_credentials_diff(
    root: &Path,
    removed: &[String],
    updated: &[(String, &CachedCredential)],
) -> Result<(), String> {
    if removed.is_empty() && updated.is_empty() {
        return Ok(());
    }
    let Some((_db, conn)) = open_or_recreate(root) else {
        return Ok(());
    };
    conn.execute("BEGIN IMMEDIATE")
        .map_err(|err| err.to_string())?;
    let outcome = (|| -> Result<(), String> {
        let mut del_cred = conn
            .prepare("DELETE FROM vault_credentials WHERE path = ?1")
            .map_err(|err| err.to_string())?;
        for path in removed {
            del_cred.reset().map_err(|err| err.to_string())?;
            bind(&mut del_cred, 1, Value::from_text(path.clone()))?;
            del_cred.run_ignore_rows().map_err(|err| err.to_string())?;
        }
        let mut upsert_cred = conn
            .prepare(
                "INSERT OR REPLACE INTO vault_credentials (path, size, mtime_secs, mtime_nanos, summary_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(|err| err.to_string())?;
        for (path, entry) in updated {
            let (size, secs, nanos) = stamp_to_parts(entry.stamp);
            let summary_val = match &entry.summary {
                Some(summary) => match serde_json::to_string(summary) {
                    Ok(s) => Value::from_text(s),
                    Err(_) => Value::Null,
                },
                None => Value::Null,
            };
            upsert_cred.reset().map_err(|err| err.to_string())?;
            bind(&mut upsert_cred, 1, Value::from_text(path.clone()))?;
            bind(&mut upsert_cred, 2, Value::Numeric(Numeric::Integer(size)))?;
            bind(&mut upsert_cred, 3, Value::Numeric(Numeric::Integer(secs)))?;
            bind(&mut upsert_cred, 4, Value::Numeric(Numeric::Integer(nanos)))?;
            bind(&mut upsert_cred, 5, summary_val)?;
            upsert_cred.run_ignore_rows().map_err(|err| err.to_string())?;
        }
        Ok(())
    })();

    match outcome {
        Ok(()) => {
            conn.execute("COMMIT").map_err(|err| err.to_string())?;
            Ok(())
        }
        Err(err) => {
            let _ = conn.execute("ROLLBACK");
            Err(err)
        }
    }
}
