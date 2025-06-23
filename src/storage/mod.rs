pub mod lance_indexer;
pub mod text_indexer;

use anyhow::Result;
use blake3::Hasher;
use futures::TryStreamExt;
use lancedb;
use lancedb::query::{ExecutableQuery, QueryBase};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::task;

use crate::semantic::search::SemanticSearch;
use crate::types::{Chunk, ChunkConfig};
use lance_indexer::LanceIndexer;
use text_indexer::TextIndexer;

/// Combined processing service that uses both indexers
pub struct Processing {
    lance_indexer: LanceIndexer,
    text_indexer: TextIndexer,
    semantic_search: Option<SemanticSearch>,
    data_dir: PathBuf,
}

impl Processing {
    pub async fn new(data_dir: &Path) -> Result<Self> {
        fs::create_dir_all(data_dir)?;

        let lance_indexer = LanceIndexer::new(data_dir).await?;
        let text_indexer = TextIndexer::new(data_dir)?;

        Ok(Self {
            lance_indexer,
            text_indexer,
            semantic_search: None,
            data_dir: data_dir.to_path_buf(),
        })
    }

    pub async fn process_files(&mut self, files: Vec<PathBuf>) -> Result<usize> {
        let mut total_chunks = 0;
        let batch_size = (num_cpus::get() * 2).clamp(8, 32);

        let batch_futures: Vec<_> = files
            .chunks(batch_size)
            .map(|batch| {
                let batch = batch.to_vec();

                task::spawn(async move {
                    let mut batch_chunks = Vec::new();

                    let file_futures: Vec<_> = batch
                        .into_iter()
                        .map(|file_path| async move { Self::process_file(&file_path).await.ok() })
                        .collect();

                    let file_results = futures::future::join_all(file_futures).await;

                    for chunks in file_results.into_iter().flatten() {
                        if !chunks.is_empty() {
                            batch_chunks.extend(chunks);
                        }
                    }

                    Ok::<Vec<Chunk>, anyhow::Error>(batch_chunks)
                })
            })
            .collect();

        let batch_results = futures::future::join_all(batch_futures).await;
        let mut all_chunks = Vec::new();

        for chunks in batch_results.into_iter().flatten().flatten() {
            total_chunks += chunks.len();
            all_chunks.extend(chunks);
        }

        if !all_chunks.is_empty() {
            if let Err(e) = self.lance_indexer.index_chunks(&all_chunks).await {
                eprintln!("Warning: Failed to index chunks in LanceDB: {}", e);
            }

            if let Err(e) = self.text_indexer.index_chunks(&all_chunks) {
                eprintln!("Warning: Failed to index chunks in Tantivy: {}", e);
            }
        }

        Ok(total_chunks)
    }

    async fn process_file(file_path: &Path) -> Result<Vec<Chunk>> {
        let metadata = fs::metadata(file_path)?;

        let content = if metadata.len() > 1_000_000 {
            task::spawn_blocking({
                let file_path = file_path.to_owned();
                move || {
                    use std::fs::File;
                    use std::io::Read;
                    let mut file = File::open(file_path)?;
                    let mut content = String::new();
                    file.read_to_string(&mut content)?;
                    Ok::<String, std::io::Error>(content)
                }
            })
            .await??
        } else {
            task::spawn_blocking({
                let file_path = file_path.to_owned();
                move || fs::read_to_string(file_path)
            })
            .await??
        };

        let mut hasher = Hasher::new();
        hasher.update(content.as_bytes());
        let file_hash = hasher.finalize().to_hex().to_string();

        let chunks = Self::chunk(file_path, &content, &file_hash, &ChunkConfig::default());
        Ok(chunks)
    }

    fn chunk(
        file_path: &Path,
        content: &str,
        file_hash: &str,
        chunk_config: &ChunkConfig,
    ) -> Vec<Chunk> {
        let mut chunks = Vec::new();
        let content_len = content.len();

        if content_len < chunk_config.min_chunk_size {
            return chunks;
        }

        let mut chunk_id = 0;
        let mut start_byte = 0;

        while start_byte < content_len {
            let target_end = (start_byte + chunk_config.chunk_size).min(content_len);
            let mut actual_end = target_end;

            while actual_end > start_byte && !content.is_char_boundary(actual_end) {
                actual_end -= 1;
            }

            if actual_end < content_len {
                if let Some(slice) = content.get(start_byte..actual_end) {
                    if let Some(newline_pos) = slice.rfind('\n') {
                        let newline_byte_pos = start_byte + newline_pos + 1;
                        if content.is_char_boundary(newline_byte_pos) {
                            actual_end = newline_byte_pos;
                        }
                    }
                }
            }

            if actual_end - start_byte < chunk_config.min_chunk_size && start_byte > 0 {
                break;
            }

            let chunk_content = match content.get(start_byte..actual_end) {
                Some(slice) => slice,
                None => break,
            };

            let start_line = content[..start_byte].matches('\n').count() + 1;
            let end_line = start_line + chunk_content.matches('\n').count();

            let mut hasher = Hasher::new();
            hasher.update(chunk_content.as_bytes());
            let chunk_hash = hasher.finalize().to_hex().to_string();

            let chunk = Chunk {
                id: format!("{}:{}", file_hash, chunk_id),
                file_path: file_path.to_owned(),
                start_line,
                end_line,
                content: chunk_content.to_string(),
                hash: chunk_hash,
            };

            chunks.push(chunk);
            chunk_id += 1;

            let overlap = chunk_config.overlap_size.min((actual_end - start_byte) / 2);
            let next_start = actual_end.saturating_sub(overlap);

            let mut safe_next_start = next_start;
            while safe_next_start > start_byte && !content.is_char_boundary(safe_next_start) {
                safe_next_start -= 1;
            }

            if safe_next_start <= start_byte {
                start_byte = actual_end;
            } else {
                start_byte = safe_next_start;
            }
        }

        chunks
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<(Chunk, f32)>> {
        let query = query.trim();

        if query.starts_with('\'') {
            return self.text_search(&query[1..], limit).await;
        }

        let chunks = self.lance_indexer.search(query, limit).await?;
        Ok(chunks.into_iter().map(|c| (c, 1.0)).collect())
    }

    pub async fn text_search(&self, query: &str, limit: usize) -> Result<Vec<(Chunk, f32)>> {
        if query.is_empty() {
            return Ok(Vec::new());
        }

        self.text_indexer.search(query, limit)
    }

    pub fn init_semantic_search(&mut self) -> Result<()> {
        match SemanticSearch::new(&self.data_dir, 0) {
            Ok(search) => {
                self.semantic_search = Some(search);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    pub fn has_semantic_search(&self) -> bool {
        self.semantic_search.is_some()
    }

    pub async fn close(mut self) {
        let _ = self.text_indexer.commit();
    }

    pub async fn add_chunks_to_lancedb(&mut self, chunks: &[Chunk]) -> Result<()> {
        self.lance_indexer.index_chunks(chunks).await
    }

    pub async fn lance_search(&self, query: &str, limit: usize) -> Result<Vec<Chunk>> {
        self.lance_indexer.search(query, limit).await
    }
}

#[derive(Clone)]
pub struct LanceDBStorage {
    connection: Option<Arc<lancedb::Connection>>,
    table: Option<Arc<lancedb::Table>>,
}

impl LanceDBStorage {
    pub async fn new(data_dir: &Path) -> Result<Self> {
        let db_path = data_dir.join("lancedb_chunks");
        std::fs::create_dir_all(&db_path)?;

        let connection = lancedb::connect(&db_path.to_string_lossy())
            .execute()
            .await?;

        Ok(Self {
            connection: Some(Arc::new(connection)),
            table: None,
        })
    }

    pub async fn init_table(&mut self) -> Result<()> {
        if let Some(ref connection) = self.connection {
            // Try to open existing table
            if let Ok(table) = connection.open_table("chunks").execute().await {
                self.table = Some(Arc::new(table));
            }
            // If table doesn't exist, it will be created when first chunks are added
        }
        Ok(())
    }

    pub async fn add_chunks(&mut self, chunks: &[crate::types::Chunk]) -> Result<()> {
        use arrow_array::{RecordBatch, RecordBatchIterator, StringArray, UInt64Array};
        use arrow_schema::{DataType, Field, Schema};
        use std::sync::Arc;

        if chunks.is_empty() {
            return Ok(());
        }

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("file_path", DataType::Utf8, false),
            Field::new("start_line", DataType::UInt64, false),
            Field::new("end_line", DataType::UInt64, false),
            Field::new("content", DataType::Utf8, false),
            Field::new("hash", DataType::Utf8, false),
        ]));

        let ids: Vec<String> = chunks.iter().map(|c| c.id.clone()).collect();
        let file_paths: Vec<String> = chunks
            .iter()
            .map(|c| c.file_path.to_string_lossy().to_string())
            .collect();
        let start_lines: Vec<u64> = chunks.iter().map(|c| c.start_line as u64).collect();
        let end_lines: Vec<u64> = chunks.iter().map(|c| c.end_line as u64).collect();
        let contents: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();
        let hashes: Vec<String> = chunks.iter().map(|c| c.hash.clone()).collect();

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(ids)),
                Arc::new(StringArray::from(file_paths)),
                Arc::new(UInt64Array::from(start_lines)),
                Arc::new(UInt64Array::from(end_lines)),
                Arc::new(StringArray::from(contents)),
                Arc::new(StringArray::from(hashes)),
            ],
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema.clone());

        if let Some(ref connection) = self.connection {
            if self.table.is_none() {
                let table = connection
                    .create_table("chunks", Box::new(batches))
                    .execute()
                    .await?;
                self.table = Some(Arc::new(table));
            } else {
                if let Some(ref table) = self.table {
                    table.add(Box::new(batches)).execute().await?;
                }
            }
        }

        Ok(())
    }

    pub async fn search_chunks(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<crate::types::Chunk>> {
        if let Some(ref connection) = self.connection {
            let table = match connection.open_table("chunks").execute().await {
                Ok(table) => table,
                Err(_) => return Ok(Vec::new()),
            };

            let results = table
                .query()
                .only_if(&format!("content LIKE '%{}%'", query.replace("'", "''")))
                .limit(limit)
                .execute()
                .await?;

            let batches: Vec<_> = results.try_collect().await?;
            let mut chunks = Vec::new();

            for batch in batches {
                let num_rows = batch.num_rows();
                for i in 0..num_rows {
                    if let (
                        Some(id_col),
                        Some(file_path_col),
                        Some(start_line_col),
                        Some(end_line_col),
                        Some(content_col),
                        Some(hash_col),
                    ) = (
                        batch.column_by_name("id").and_then(|col| {
                            col.as_any().downcast_ref::<arrow_array::StringArray>()
                        }),
                        batch.column_by_name("file_path").and_then(|col| {
                            col.as_any().downcast_ref::<arrow_array::StringArray>()
                        }),
                        batch.column_by_name("start_line").and_then(|col| {
                            col.as_any().downcast_ref::<arrow_array::UInt64Array>()
                        }),
                        batch.column_by_name("end_line").and_then(|col| {
                            col.as_any().downcast_ref::<arrow_array::UInt64Array>()
                        }),
                        batch.column_by_name("content").and_then(|col| {
                            col.as_any().downcast_ref::<arrow_array::StringArray>()
                        }),
                        batch.column_by_name("hash").and_then(|col| {
                            col.as_any().downcast_ref::<arrow_array::StringArray>()
                        }),
                    ) {
                        chunks.push(crate::types::Chunk {
                            id: id_col.value(i).to_string(),
                            file_path: std::path::PathBuf::from(file_path_col.value(i)),
                            start_line: start_line_col.value(i) as usize,
                            end_line: end_line_col.value(i) as usize,
                            content: content_col.value(i).to_string(),
                            hash: hash_col.value(i).to_string(),
                        });
                    }
                }
            }

            Ok(chunks)
        } else {
            Ok(Vec::new())
        }
    }
}
