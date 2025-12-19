use anyhow::Result;
use arrow_array::types::Float32Type;
use arrow_array::{FixedSizeListArray, RecordBatch, RecordBatchIterator, StringArray, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb;
use lancedb::query::{ExecutableQuery, QueryBase};
use std::path::Path;
use std::sync::Arc;

use crate::semantic::embeddings::VectorStore;
use crate::types::{Chunk, FileIndex};

pub struct LanceIndexer {
    connection: lancedb::Connection,
}

impl LanceIndexer {
    pub async fn new(data_dir: &Path) -> Result<Self> {
        let db_path = data_dir.join("lancedb_chunks");
        std::fs::create_dir_all(&db_path)?;

        let connection = lancedb::connect(&db_path.to_string_lossy())
            .execute()
            .await?;

        Ok(Self { connection })
    }

    pub async fn index_chunks(&mut self, chunks: &[Chunk]) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }

        let schema = Arc::new(Schema::new(vec![
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
        ]));

        let ids: Vec<String> = chunks.iter().map(|c| c.id.clone()).collect();
        let file_paths: Vec<String> = chunks
            .iter()
            .map(|c| c.file_path.to_string_lossy().to_string())
            .collect();
        let start_lines: Vec<u64> = chunks.iter().map(|c| c.start_line as u64).collect();
        let end_lines: Vec<u64> = chunks.iter().map(|c| c.end_line as u64).collect();
        let contents: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();

        let chunks_for_embedding: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();

        let vectors: Vec<Option<Vec<Option<f32>>>> =
            tokio::task::spawn_blocking(move || -> Result<_> {
                let mut vector_store = VectorStore::new()?;

                Ok(chunks_for_embedding
                    .iter()
                    .map(|content| {
                        vector_store
                            .generate_embedding(content)
                            .map(|embedding| embedding.into_iter().map(Some).collect())
                            .ok()
                    })
                    .collect())
            })
            .await??;

        let vector_array =
            FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(vectors, 384);

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(ids)),
                Arc::new(StringArray::from(file_paths)),
                Arc::new(UInt64Array::from(start_lines)),
                Arc::new(UInt64Array::from(end_lines)),
                Arc::new(StringArray::from(contents)),
                Arc::new(vector_array),
            ],
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema.clone());

        match self.connection.open_table("chunks").execute().await {
            Ok(table) => {
                table.add(Box::new(batches)).execute().await?;
            }
            Err(_) => {
                self.connection
                    .create_table("chunks", Box::new(batches))
                    .execute()
                    .await?;
            }
        }

        Ok(())
    }

    pub async fn search(&mut self, query: &str, limit: usize) -> Result<Vec<Chunk>> {
        let table = match self.connection.open_table("chunks").execute().await {
            Ok(table) => table,
            Err(_) => return Ok(Vec::new()),
        };

        let query_str = query.to_string();
        let query_embedding = tokio::task::spawn_blocking(move || {
            let mut vector_store = VectorStore::new().ok()?;
            vector_store.generate_embedding(&query_str).ok()
        })
        .await?;

        if let Some(query_embedding) = query_embedding {
            let results = table
                .query()
                .nearest_to(query_embedding)?
                .limit(limit)
                .execute()
                .await?;

            let batches: Vec<_> = results.try_collect().await?;
            let mut chunks = Vec::new();

            for batch in batches {
                let num_rows = batch.num_rows();
                for i in 0..num_rows {
                    if let Some(chunk) = self.extract_chunk_from_batch(&batch, i) {
                        chunks.push(chunk);
                    }
                }
            }

            return Ok(chunks);
        }

        let results = table
            .query()
            .only_if(format!("content LIKE '%{}%'", query.replace("'", "''")))
            .limit(limit)
            .execute()
            .await?;

        let batches: Vec<_> = results.try_collect().await?;
        let mut chunks = Vec::new();

        for batch in batches {
            let num_rows = batch.num_rows();
            for i in 0..num_rows {
                if let Some(chunk) = self.extract_chunk_from_batch(&batch, i) {
                    chunks.push(chunk);
                }
            }
        }

        Ok(chunks)
    }

    pub async fn get_file_index(&self, file_path: &Path) -> Result<Option<FileIndex>> {
        let file_table = match self.connection.open_table("file_index").execute().await {
            Ok(table) => table,
            Err(_) => return Ok(None),
        };

        let path_str = file_path.to_string_lossy();
        let results = file_table
            .query()
            .only_if(format!("file_path = '{}'", path_str.replace("'", "''")))
            .limit(1)
            .execute()
            .await?;

        let batches: Vec<_> = results.try_collect().await?;
        for batch in batches {
            if batch.num_rows() > 0 {
                if let Some(file_index) = self.extract_file_index_from_batch(&batch, 0) {
                    return Ok(Some(file_index));
                }
            }
        }

        Ok(None)
    }

    pub async fn update_file_index(&mut self, file_path: &Path, file_hash: &str) -> Result<()> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("file_path", DataType::Utf8, false),
            Field::new("hash", DataType::Utf8, false),
        ]));

        let file_index = FileIndex {
            file_path: file_path.to_owned(),
            hash: file_hash.to_string(),
        };

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![
                    file_index.file_path.to_string_lossy().to_string(),
                ])),
                Arc::new(StringArray::from(vec![file_index.hash])),
            ],
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema.clone());

        let path_str = file_path.to_string_lossy();
        if let Ok(file_table) = self.connection.open_table("file_index").execute().await {
            if (file_table
                .delete(&format!("file_path = '{}'", path_str.replace("'", "''")))
                .await)
                .is_ok()
            {
                file_table.add(Box::new(batches)).execute().await?;
            }
        } else {
            let _table = self
                .connection
                .create_table("file_index", Box::new(batches))
                .execute()
                .await?;
        }

        Ok(())
    }

    pub async fn remove_file_chunks(&mut self, file_path: &Path) -> Result<()> {
        if let Ok(chunks_table) = self.connection.open_table("chunks").execute().await {
            let path_str = file_path.to_string_lossy();
            chunks_table
                .delete(&format!("file_path = '{}'", path_str.replace("'", "''")))
                .await?;
        }

        if let Ok(file_table) = self.connection.open_table("file_index").execute().await {
            let path_str = file_path.to_string_lossy();
            file_table
                .delete(&format!("file_path = '{}'", path_str.replace("'", "''")))
                .await?;
        }

        Ok(())
    }

    fn extract_chunk_from_batch(&self, batch: &RecordBatch, row_index: usize) -> Option<Chunk> {
        let id_col = batch
            .column_by_name("id")?
            .as_any()
            .downcast_ref::<StringArray>()?;
        let file_path_col = batch
            .column_by_name("file_path")?
            .as_any()
            .downcast_ref::<StringArray>()?;
        let start_line_col = batch
            .column_by_name("start_line")?
            .as_any()
            .downcast_ref::<UInt64Array>()?;
        let end_line_col = batch
            .column_by_name("end_line")?
            .as_any()
            .downcast_ref::<UInt64Array>()?;
        let content_col = batch
            .column_by_name("content")?
            .as_any()
            .downcast_ref::<StringArray>()?;

        Some(Chunk {
            id: id_col.value(row_index).to_string(),
            file_path: std::path::PathBuf::from(file_path_col.value(row_index)),
            start_line: start_line_col.value(row_index) as usize,
            end_line: end_line_col.value(row_index) as usize,
            content: content_col.value(row_index).to_string(),
        })
    }

    fn extract_file_index_from_batch(
        &self,
        batch: &RecordBatch,
        row_index: usize,
    ) -> Option<FileIndex> {
        let file_path_col = batch
            .column_by_name("file_path")?
            .as_any()
            .downcast_ref::<StringArray>()?;
        let hash_col = batch
            .column_by_name("hash")?
            .as_any()
            .downcast_ref::<StringArray>()?;

        Some(FileIndex {
            file_path: std::path::PathBuf::from(file_path_col.value(row_index)),
            hash: hash_col.value(row_index).to_string(),
        })
    }
}
