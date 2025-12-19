use anyhow::Result;
use arrow_array::types::Float32Type;
use arrow_array::{FixedSizeListArray, RecordBatch, RecordBatchIterator, StringArray, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::semantic::embeddings::VectorStore;
use crate::types::{Chunk, FileIndex};

const BATCH_SIZE: usize = 100;

fn create_chunks_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("file_path", DataType::Utf8, false),
        Field::new("start_line", DataType::UInt64, false),
        Field::new("end_line", DataType::UInt64, false),
        Field::new("content", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 384),
            true,
        ),
    ]))
}

fn create_file_index_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("file_path", DataType::Utf8, false),
        Field::new("hash", DataType::Utf8, false),
    ]))
}

pub struct LanceIndexer {
    conn: lancedb::Connection,
}

impl LanceIndexer {
    pub async fn new(data_dir: &Path) -> Result<Self> {
        let db_path = data_dir.join("lancedb_chunks");
        std::fs::create_dir_all(&db_path)?;
        let conn = lancedb::connect(&db_path.to_string_lossy())
            .execute()
            .await?;
        Ok(Self { conn })
    }

    pub async fn index_chunks(&mut self, chunks: &[Chunk]) -> Result<()> {
        for batch in chunks.chunks(BATCH_SIZE) {
            self.index_batch(batch).await?;
        }
        Ok(())
    }

    async fn index_batch(&mut self, chunks: &[Chunk]) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }

        let ids: Vec<_> = chunks.iter().map(|c| c.id.clone()).collect();
        let paths: Vec<_> = chunks
            .iter()
            .map(|c| c.file_path.to_string_lossy().into_owned())
            .collect();
        let starts: Vec<_> = chunks.iter().map(|c| c.start_line as u64).collect();
        let ends: Vec<_> = chunks.iter().map(|c| c.end_line as u64).collect();
        let contents: Vec<_> = chunks.iter().map(|c| c.content.clone()).collect();

        let vectors = tokio::task::spawn_blocking({
            let contents = contents.clone();
            move || -> Result<Vec<Option<Vec<Option<f32>>>>> {
                use rayon::prelude::*;
                Ok(contents
                    .par_iter()
                    .map(|content| {
                        VectorStore::new()
                            .ok()?
                            .generate_embedding(content)
                            .map(|e| e.into_iter().map(Some).collect())
                            .ok()
                    })
                    .collect())
            }
        })
        .await??;

        let schema = create_chunks_schema();
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(ids)),
                Arc::new(StringArray::from(paths)),
                Arc::new(UInt64Array::from(starts)),
                Arc::new(UInt64Array::from(ends)),
                Arc::new(StringArray::from(contents)),
                Arc::new(
                    FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(vectors, 384),
                ),
            ],
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema);
        match self.conn.open_table("chunks").execute().await {
            Ok(t) => {
                t.add(Box::new(batches)).execute().await?;
            }
            Err(_) => {
                self.conn
                    .create_table("chunks", Box::new(batches))
                    .execute()
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn search(&mut self, query: &str, limit: usize) -> Result<Vec<Chunk>> {
        let table = match self.conn.open_table("chunks").execute().await {
            Ok(t) => t,
            Err(_) => return Ok(Vec::new()),
        };

        let query_str = query.to_string();
        let embedding = tokio::task::spawn_blocking(move || {
            VectorStore::new().ok()?.generate_embedding(&query_str).ok()
        })
        .await?;

        let batches: Vec<RecordBatch> = if let Some(emb) = embedding {
            table
                .query()
                .nearest_to(emb)?
                .limit(limit)
                .execute()
                .await?
                .try_collect()
                .await?
        } else {
            let filter = format!("content LIKE '%{}%'", query.replace("'", "''"));
            table
                .query()
                .only_if(filter)
                .limit(limit)
                .execute()
                .await?
                .try_collect()
                .await?
        };

        Ok(batches
            .iter()
            .flat_map(|b| (0..b.num_rows()).filter_map(|i| extract_chunk(b, i)))
            .collect())
    }

    pub async fn get_file_index(&self, file_path: &Path) -> Result<Option<FileIndex>> {
        let table = match self.conn.open_table("file_index").execute().await {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };

        let filter = format!("file_path = {}", escape_sql(&file_path.to_string_lossy()));
        let batches: Vec<_> = table
            .query()
            .only_if(filter)
            .limit(1)
            .execute()
            .await?
            .try_collect()
            .await?;

        Ok(batches.first().and_then(|b| extract_file_index(b, 0)))
    }

    pub async fn update_file_index(&mut self, file_path: &Path, hash: &str) -> Result<()> {
        let path_str = file_path.to_string_lossy().into_owned();
        let filter = format!("file_path = {}", escape_sql(&path_str));

        if let Ok(table) = self.conn.open_table("file_index").execute().await {
            let _ = table.delete(&filter).await;
        }

        let schema = create_file_index_schema();
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![path_str])),
                Arc::new(StringArray::from(vec![hash.to_string()])),
            ],
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema);
        match self.conn.open_table("file_index").execute().await {
            Ok(t) => {
                t.add(Box::new(batches)).execute().await?;
            }
            Err(_) => {
                self.conn
                    .create_table("file_index", Box::new(batches))
                    .execute()
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn remove_file_chunks(&mut self, file_path: &Path) -> Result<()> {
        let filter = format!("file_path = {}", escape_sql(&file_path.to_string_lossy()));

        if let Ok(t) = self.conn.open_table("chunks").execute().await {
            t.delete(&filter).await?;
        }
        if let Ok(t) = self.conn.open_table("file_index").execute().await {
            t.delete(&filter).await?;
        }
        Ok(())
    }
}

fn extract_chunk(batch: &RecordBatch, i: usize) -> Option<Chunk> {
    let id_col = batch
        .column_by_name("id")?
        .as_any()
        .downcast_ref::<StringArray>()?;
    let path_col = batch
        .column_by_name("file_path")?
        .as_any()
        .downcast_ref::<StringArray>()?;
    let start_col = batch
        .column_by_name("start_line")?
        .as_any()
        .downcast_ref::<UInt64Array>()?;
    let end_col = batch
        .column_by_name("end_line")?
        .as_any()
        .downcast_ref::<UInt64Array>()?;
    let content_col = batch
        .column_by_name("content")?
        .as_any()
        .downcast_ref::<StringArray>()?;

    Some(Chunk {
        id: id_col.value(i).to_string(),
        file_path: PathBuf::from(path_col.value(i)),
        start_line: start_col.value(i) as usize,
        end_line: end_col.value(i) as usize,
        content: content_col.value(i).to_string(),
    })
}

fn extract_file_index(batch: &RecordBatch, i: usize) -> Option<FileIndex> {
    let path_col = batch
        .column_by_name("file_path")?
        .as_any()
        .downcast_ref::<StringArray>()?;
    let hash_col = batch
        .column_by_name("hash")?
        .as_any()
        .downcast_ref::<StringArray>()?;

    Some(FileIndex {
        file_path: PathBuf::from(path_col.value(i)),
        hash: hash_col.value(i).to_string(),
    })
}

fn escape_sql(s: &str) -> String {
    format!("'{}'", s.replace("'", "''"))
}
