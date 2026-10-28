// SPDX-License-Identifier: MIT

use super::result::{ContentLocator, ExtractedSegment, FileRevision, MatchReason, SearchHit};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS files (
 id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, root TEXT NOT NULL,
 revision TEXT NOT NULL, size INTEGER NOT NULL, modified_ns INTEGER NOT NULL,
 state TEXT NOT NULL, reason TEXT, identity TEXT, seen INTEGER NOT NULL DEFAULT 0, next INTEGER
);
CREATE TABLE IF NOT EXISTS segments (
 id INTEGER PRIMARY KEY, file INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
 ordinal INTEGER NOT NULL, locator TEXT NOT NULL, text TEXT NOT NULL, modality TEXT NOT NULL,
 vector BLOB, fingerprint TEXT, UNIQUE(file,ordinal)
);
CREATE VIRTUAL TABLE IF NOT EXISTS segment_fts USING fts5(name, body, tokenize='unicode61 remove_diacritics 2');
CREATE TRIGGER IF NOT EXISTS segments_delete AFTER DELETE ON segments BEGIN
 DELETE FROM segment_fts WHERE rowid=old.id;
END;
CREATE INDEX IF NOT EXISTS files_root ON files(root);
CREATE INDEX IF NOT EXISTS segments_file ON segments(file);
CREATE TABLE IF NOT EXISTS index_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
PRAGMA user_version=2;
"#;

pub(super) fn connect(path: &Path, write: bool) -> Result<Connection, String> {
    if let Some(parent) = path.parent()
        && write
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let flags = if write {
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
    } else {
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let connection = Connection::open_with_flags(path, flags).map_err(|error| error.to_string())?;
    connection
        .busy_timeout(if write {
            Duration::from_millis(500)
        } else {
            Duration::from_millis(30)
        })
        .map_err(|error| error.to_string())?;
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(|error| error.to_string())?;
    if write {
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| error.to_string())?;
        connection
            .pragma_update(None, "synchronous", "NORMAL")
            .map_err(|error| error.to_string())?;
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if version > 2 {
            return Err("Unsupported content index version".into());
        }
        let migration = if version == 1 {
            "ALTER TABLE files ADD COLUMN identity TEXT;"
        } else {
            ""
        };
        connection
            .execute_batch(&format!("BEGIN IMMEDIATE;{migration}{SCHEMA}COMMIT;"))
            .map_err(|error| error.to_string())?;
    }
    Ok(connection)
}

pub(super) fn current(
    connection: &Connection,
    path: &Path,
    revision: &FileRevision,
    fingerprint: Option<&str>,
) -> Result<bool, String> {
    if let Some(identity) = identity(path) {
        let old: Option<(i64, String)> = connection
            .query_row(
                "SELECT id,path FROM files WHERE identity=?1 AND revision=?2 LIMIT 1",
                params![identity, revision.key()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some((id, old)) = old
            && Path::new(&old) != path
            && !Path::new(&old).try_exists().unwrap_or(true)
        {
            let transaction = connection
                .unchecked_transaction()
                .map_err(|error| error.to_string())?;
            transaction
                .execute(
                    "UPDATE files SET path=?1 WHERE id=?2",
                    params![path.to_string_lossy(), id],
                )
                .map_err(|error| error.to_string())?;
            transaction.execute("UPDATE segment_fts SET name=?1 WHERE rowid IN(SELECT id FROM segments WHERE file=?2)",params![path.file_name().unwrap_or_default().to_string_lossy(),id]).map_err(|error|error.to_string())?;
            transaction.commit().map_err(|error| error.to_string())?;
        }
    }
    let state: Option<(String,String,Option<u32>,i64)> = connection.query_row(
        "SELECT revision,state,next,EXISTS(SELECT 1 FROM segments WHERE file=files.id AND (vector IS NULL OR fingerprint IS NULL OR fingerprint != ?2)) FROM files WHERE path=?1",
        params![path.to_string_lossy(),fingerprint.unwrap_or("")], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional().map_err(|error| error.to_string())?;
    Ok(state.is_some_and(|(key, state, next, missing)| {
        key == revision.key()
            && state == "indexed"
            && next.is_none()
            && (fingerprint.is_none() || missing == 0)
    }))
}

#[expect(
    clippy::too_many_arguments,
    reason = "one transaction commits the file revision, resumable segments and compatible embeddings"
)]
pub(super) fn append(
    connection: &mut Connection,
    root: &Path,
    path: &Path,
    revision: &FileRevision,
    segments: &[ExtractedSegment],
    vectors: &[Option<Vec<f32>>],
    fingerprint: Option<&str>,
    next: Option<u32>,
    append: bool,
) -> Result<(), String> {
    let identity = identity(path);
    if let Some(identity) = identity.as_ref() {
        let previous: Option<(i64, String)> = connection
            .query_row(
                "SELECT id,path FROM files WHERE identity=?1 AND revision=?2 LIMIT 1",
                params![identity, revision.key()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some((id, old)) = previous
            && Path::new(&old) != path
            && !Path::new(&old).try_exists().unwrap_or(true)
        {
            let transaction = connection
                .transaction()
                .map_err(|error| error.to_string())?;
            transaction
                .execute(
                    "UPDATE files SET path=?1,root=?2 WHERE id=?3",
                    params![path.to_string_lossy(), root.to_string_lossy(), id],
                )
                .map_err(|error| error.to_string())?;
            transaction.execute("UPDATE segment_fts SET name=?1 WHERE rowid IN(SELECT id FROM segments WHERE file=?2)",params![path.file_name().unwrap_or_default().to_string_lossy(),id]).map_err(|error|error.to_string())?;
            transaction.commit().map_err(|error| error.to_string())?;
        }
    }
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    transaction.execute("INSERT INTO files(path,root,revision,size,modified_ns,state,next) VALUES(?1,?2,?3,?4,?5,'indexed',?6) ON CONFLICT(path) DO UPDATE SET root=excluded.root,revision=excluded.revision,size=excluded.size,modified_ns=excluded.modified_ns,state='indexed',reason=NULL,next=excluded.next",
        params![path.to_string_lossy(),root.to_string_lossy(),revision.key(),revision.size as i64,revision.modified_ns.min(i64::MAX as u64) as i64,next]).map_err(|error| error.to_string())?;
    let id: i64 = transaction
        .query_row(
            "SELECT id FROM files WHERE path=?1",
            params![path.to_string_lossy()],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE files SET identity=?1 WHERE id=?2",
            params![identity, id],
        )
        .map_err(|error| error.to_string())?;
    if !append {
        transaction
            .execute("DELETE FROM segments WHERE file=?1", params![id])
            .map_err(|error| error.to_string())?;
    }
    let base: u32 = transaction
        .query_row(
            "SELECT coalesce(max(ordinal)+1,0) FROM segments WHERE file=?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    for (ordinal, segment) in segments.iter().enumerate() {
        let vector: Option<Vec<u8>> = vectors.get(ordinal).and_then(Option::as_ref).map(|vector| {
            vector
                .iter()
                .flat_map(|value| half::f16::from_f32(*value).to_le_bytes())
                .collect()
        });
        transaction.execute("INSERT INTO segments(file,ordinal,locator,text,modality,vector,fingerprint) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![id,i64::from(base)+ordinal as i64,serde_json::to_string(&segment.locator).map_err(|error| error.to_string())?,segment.text,segment.modality,vector,fingerprint]).map_err(|error| error.to_string())?;
        let segment_id = transaction.last_insert_rowid();
        transaction
            .execute(
                "INSERT INTO segment_fts(rowid,name,body) VALUES(?1,?2,?3)",
                params![
                    segment_id,
                    super::hybrid::folded(&name),
                    super::hybrid::folded(&segment.text)
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

pub(super) fn cursor(
    connection: &Connection,
    path: &Path,
    revision: &FileRevision,
) -> Result<u32, String> {
    let row: Option<(String, Option<u32>)> = connection
        .query_row(
            "SELECT revision,next FROM files WHERE path=?1",
            params![path.to_string_lossy()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(row
        .filter(|(known, _)| *known == revision.key())
        .and_then(|(_, cursor)| cursor)
        .unwrap_or(0))
}

pub(super) fn mark(
    connection: &Connection,
    root: &Path,
    path: &Path,
    revision: &FileRevision,
    reason: &str,
) -> Result<(), String> {
    connection.execute("INSERT INTO files(path,root,revision,size,modified_ns,state,reason) VALUES(?1,?2,?3,?4,?5,'skipped',?6) ON CONFLICT(path) DO UPDATE SET root=excluded.root,revision=excluded.revision,size=excluded.size,modified_ns=excluded.modified_ns,state='skipped',reason=excluded.reason,next=NULL",params![path.to_string_lossy(),root.to_string_lossy(),revision.key(),revision.size as i64,revision.modified_ns.min(i64::MAX as u64) as i64,reason]).map_err(|error| error.to_string())?;
    Ok(())
}

fn hit(row: &rusqlite::Row<'_>) -> rusqlite::Result<SearchHit> {
    let locator: String = row.get(5)?;
    let locator: ContentLocator = serde_json::from_str(&locator).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(SearchHit {
        file_id: row.get(0)?,
        segment_id: row.get(1)?,
        path: PathBuf::from(row.get::<_, String>(2)?),
        revision: FileRevision {
            size: row.get::<_, i64>(3)?.max(0) as u64,
            modified_ns: row.get::<_, i64>(4)?.max(0) as u64,
        },
        locator,
        excerpt: row.get::<_, String>(6)?,
        modality: row.get(7)?,
        reason: MatchReason::Keyword,
        score: 0,
        weak: false,
    })
}

pub(super) fn keyword(
    connection: &Connection,
    query: &str,
    roots: &[PathBuf],
    code: bool,
    kind: super::result::SearchKind,
) -> Result<Vec<SearchHit>, String> {
    scope_function(connection, roots, kind)?;
    let tokens: Vec<_> = query
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|token| !token.is_empty())
        .take(32)
        .collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    let expression = tokens
        .iter()
        .enumerate()
        .map(|(index, token)| {
            format!(
                "\"{}\"{}",
                super::hybrid::folded(token).replace('"', "\"\""),
                if index + 1 == tokens.len() { "*" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(" OR ");
    let mut statement = connection.prepare("SELECT f.id,s.id,f.path,f.size,f.modified_ns,s.locator,s.text,s.modality,bm25(segment_fts,3.0,1.0) FROM segment_fts JOIN segments s ON s.id=segment_fts.rowid JOIN files f ON f.id=s.file WHERE segment_fts MATCH ?1 AND f.state='indexed' AND strata_scope(f.path) ORDER BY bm25(segment_fts,3.0,1.0) LIMIT 1000").map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![expression], hit)
        .map_err(|error| error.to_string())?;
    let mut hits = Vec::new();
    for hit in rows {
        let mut hit = hit.map_err(|error| error.to_string())?;
        if !roots
            .iter()
            .any(|root| super::policy::within(&hit.path, root))
            || (hit.modality == "code" && !code)
        {
            continue;
        }
        hit.score = super::hybrid::keyword_score(query, &hit);
        hits.push(hit);
    }
    hits.sort_by_key(|hit| std::cmp::Reverse(hit.score));
    hits.truncate(100);
    Ok(hits)
}

pub(super) struct VectorIndex {
    pub hits: Vec<SearchHit>,
    pub values: Vec<half::f16>,
}
pub(super) fn vectors(
    connection: &Connection,
    fingerprint: &str,
    roots: &[PathBuf],
    code: bool,
    kind: super::result::SearchKind,
) -> Result<VectorIndex, String> {
    scope_function(connection, roots, kind)?;
    let mut statement = connection.prepare("SELECT f.id,s.id,f.path,f.size,f.modified_ns,s.locator,substr(s.text,1,240),s.modality,s.vector FROM segments s JOIN files f ON f.id=s.file WHERE f.state='indexed' AND s.fingerprint=?1 AND length(s.vector)=1536 AND strata_scope(f.path) LIMIT 80000").map_err(|error| error.to_string())?;
    let mut rows = statement
        .query(params![fingerprint])
        .map_err(|error| error.to_string())?;
    let mut index = VectorIndex {
        hits: Vec::new(),
        values: Vec::new(),
    };
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        let hit = hit(row).map_err(|error| error.to_string())?;
        if !roots
            .iter()
            .any(|root| super::policy::within(&hit.path, root))
            || (hit.modality == "code" && !code)
        {
            continue;
        }
        let bytes: Vec<u8> = row.get(8).map_err(|error| error.to_string())?;
        let values: Vec<_> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| half::f16::from_le_bytes([bytes[0], bytes[1]]))
            .collect();
        if values.iter().any(|value| !value.is_finite()) {
            continue;
        }
        index.values.extend(values);
        index.hits.push(hit);
    }
    Ok(index)
}

pub(super) fn reconcile(
    connection: &Connection,
    roots: &[PathBuf],
    seen: &std::collections::HashSet<PathBuf>,
    complete: &[PathBuf],
) -> Result<(), String> {
    let mut statement = connection
        .prepare("SELECT path,root FROM files")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                PathBuf::from(row.get::<_, String>(0)?),
                PathBuf::from(row.get::<_, String>(1)?),
            ))
        })
        .map_err(|error| error.to_string())?;
    let removed: Vec<_> = rows
        .filter_map(Result::ok)
        .filter(|(path, _root)| {
            !roots
                .iter()
                .any(|candidate| super::policy::within(path, candidate))
                || (complete
                    .iter()
                    .any(|candidate| super::policy::within(path, candidate))
                    && !seen.contains(path))
        })
        .map(|(path, _)| path)
        .collect();
    drop(statement);
    for path in removed {
        connection
            .execute(
                "DELETE FROM files WHERE path=?1",
                params![path.to_string_lossy()],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;

fn identity(path: &Path) -> Option<String> {
    #[cfg(windows)]
    {
        strata_win_shell::file_identity(path)
            .ok()
            .map(|(volume, file)| format!("{volume:x}:{file:x}"))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path)
            .ok()
            .map(|metadata| format!("{:x}:{:x}", metadata.dev(), metadata.ino()))
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = path;
        None
    }
}

fn scope_function(
    connection: &Connection,
    roots: &[PathBuf],
    kind: super::result::SearchKind,
) -> Result<(), String> {
    let roots = roots.to_vec();
    connection
        .create_scalar_function(
            "strata_scope",
            1,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
            move |context| {
                let path = context.get::<String>(0)?;
                Ok(kind.accepts(Path::new(&path))
                    && roots
                        .iter()
                        .any(|root| super::policy::within(Path::new(&path), root)))
            },
        )
        .map_err(|error| error.to_string())
}
