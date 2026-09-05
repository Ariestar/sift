//! SQLite-backed embedding index for semantic and hybrid search.

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

/// Ensure the archive has one embedding index for the configured model.
/// Changing model or dimensions discards only derived vectors; records stay
/// untouched and the caller fills the index from their content hashes.
pub fn ensure_index(conn: &Connection, model: &str, dimensions: usize) -> Result<()> {
    if model.trim().is_empty() || dimensions == 0 {
        bail!("embedding index requires a model and positive dimensions");
    }
    let dimensions =
        i64::try_from(dimensions).context("embedding dimensions exceed SQLite range")?;
    let existing: Option<(String, i64)> = conn
        .query_row(
            "SELECT model, dimensions FROM embedding_state WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .context("failed to read embedding index state")?;
    if existing
        .as_ref()
        .is_some_and(|(current_model, current_dimensions)| {
            current_model == model && *current_dimensions == dimensions
        })
    {
        return Ok(());
    }

    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM record_embeddings", [])?;
    tx.execute(
        "INSERT INTO embedding_state (id, model, dimensions) VALUES (1, ?1, ?2)
         ON CONFLICT(id) DO UPDATE SET model = excluded.model,
             dimensions = excluded.dimensions",
        params![model, dimensions],
    )?;
    tx.commit().context("failed to reset embedding index")?;
    Ok(())
}

pub fn upsert_embeddings(
    conn: &Connection,
    embeddings: &[(String, String, Vec<f32>)],
) -> Result<()> {
    let dimensions = index_dimensions(conn)?;
    let tx = conn.unchecked_transaction()?;
    for (record_ref, content_hash, vector) in embeddings {
        if vector.len() != dimensions {
            bail!(
                "embedding for {record_ref} has {} dimensions; expected {}",
                vector.len(),
                dimensions
            );
        }
        if vector.iter().any(|value| !value.is_finite()) {
            bail!("embedding for {record_ref} contains a non-finite value");
        }
        tx.execute(
            "INSERT INTO record_embeddings (record_ref, content_hash, vector)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(record_ref) DO UPDATE SET
                 content_hash = excluded.content_hash, vector = excluded.vector",
            params![record_ref, content_hash, vector_to_blob(vector),],
        )?;
    }
    tx.commit().context("failed to store embeddings")?;
    Ok(())
}

pub fn load_embeddings(conn: &Connection) -> Result<HashMap<String, (String, Vec<f32>)>> {
    let dimensions = index_dimensions(conn)?;
    let mut stmt = conn.prepare(
        "SELECT record_ref, content_hash, vector FROM record_embeddings
         ORDER BY record_ref",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Vec<u8>>(2)?,
        ))
    })?;
    let mut embeddings = HashMap::new();
    for row in rows {
        let (record_ref, hash, blob) = row?;
        let vector = blob_to_vector(&blob)?;
        if vector.len() != dimensions {
            bail!(
                "stored embedding for {record_ref} has {} dimensions; expected {}",
                vector.len(),
                dimensions
            );
        }
        embeddings.insert(record_ref, (hash, vector));
    }
    Ok(embeddings)
}

fn index_dimensions(conn: &Connection) -> Result<usize> {
    let dimensions: i64 = conn
        .query_row(
            "SELECT dimensions FROM embedding_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .context("embedding index is not initialized")?;
    usize::try_from(dimensions).context("embedding index dimensions are invalid")
}

fn vector_to_blob(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn blob_to_vector(blob: &[u8]) -> Result<Vec<f32>> {
    if !blob.len().is_multiple_of(std::mem::size_of::<f32>()) {
        bail!("embedding blob length is not a multiple of four bytes");
    }
    let (chunks, remainder) = blob.as_chunks::<4>();
    debug_assert!(remainder.is_empty());
    Ok(chunks
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect())
}

pub fn text_hash(text: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(text.as_bytes());
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> Result<f32> {
    if left.len() != right.len() {
        bail!("embedding dimensions do not match");
    }
    let (dot, left_norm, right_norm) = left.iter().zip(right).fold(
        (0.0_f64, 0.0_f64, 0.0_f64),
        |(dot, left_norm, right_norm), (left, right)| {
            let left = f64::from(*left);
            let right = f64::from(*right);
            (
                dot + left * right,
                left_norm + left * left,
                right_norm + right * right,
            )
        },
    );
    if left_norm == 0.0 || right_norm == 0.0 {
        return Ok(0.0);
    }
    Ok((dot / (left_norm.sqrt() * right_norm.sqrt())) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_reset_and_vector_round_trip() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE embedding_state (
                id INTEGER PRIMARY KEY, model TEXT NOT NULL, dimensions INTEGER NOT NULL
             );
             CREATE TABLE record_embeddings (
                record_ref TEXT PRIMARY KEY, content_hash TEXT NOT NULL, vector BLOB NOT NULL
             );",
        )
        .unwrap();
        ensure_index(&conn, "model", 2).unwrap();
        upsert_embeddings(&conn, &[("r1".into(), "hash".into(), vec![1.0, 2.0])]).unwrap();
        let loaded = load_embeddings(&conn).unwrap();
        assert_eq!(loaded["r1"].1, vec![1.0, 2.0]);
        ensure_index(&conn, "other-model", 2).unwrap();
        assert!(load_embeddings(&conn).unwrap().is_empty());
        assert!(
            (cosine_similarity(&[1.0, 0.0], &[1.0, 1.0]).unwrap()
                - std::f32::consts::FRAC_1_SQRT_2)
                .abs()
                < 0.001
        );
    }
}
