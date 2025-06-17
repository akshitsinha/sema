use anyhow::Result;
use blake3::Hasher;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tantivy::{
    Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument,
    collector::TopDocs,
    directory::MmapDirectory,
    doc,
    query::QueryParser,
    schema::{Field, OwnedValue, STORED, Schema, TEXT},
};
use tokio::task;

use super::Database;
use crate::types::{Chunk, ChunkConfig, FileIndex};

pub struct ProcessingService {
    db: Database,
    index: Index,
    index_writer: IndexWriter,
    index_reader: IndexReader,
    content_field: Field,
    path_field: Field,
    start_line_field: Field,
    end_line_field: Field,
}

impl ProcessingService {
    pub async fn new(data_dir: &Path, _chunk_config: ChunkConfig) -> Result<Self> {
        // Create directories
        let db_path = data_dir.join("db");
        let index_path = data_dir.join("index");

        fs::create_dir_all(&db_path)?;
        fs::create_dir_all(&index_path)?;

        // Initialize RocksDB
        let db = Database::new(&db_path)?;

        // Initialize Tantivy schema
        let mut schema_builder = Schema::builder();
        let content_field = schema_builder.add_text_field("content", TEXT);
        let path_field = schema_builder.add_text_field("path", TEXT | STORED);
        let start_line_field = schema_builder.add_u64_field("start_line", STORED);
        let end_line_field = schema_builder.add_u64_field("end_line", STORED);
        let schema = schema_builder.build();

        // Initialize Tantivy index
        let index_dir = MmapDirectory::open(&index_path)?;
        let index = Index::open_or_create(index_dir, schema.clone())?;

        let index_writer = index.writer(200_000_000)?; // 200MB heap - larger for better performance
        let index_reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;

        Ok(Self {
            db,
            index,
            index_writer,
            index_reader,
            content_field,
            path_field,
            start_line_field,
            end_line_field,
        })
    }

    pub async fn process_files(
        &mut self,
        files: Vec<PathBuf>,
        max_file_size: u64,
    ) -> Result<usize> {
        let mut total_chunks = 0;

        // Load already indexed files in parallel
        let file_check_futures: Vec<_> = files
            .iter()
            .map(|file_path| {
                let path = file_path.clone();
                let db_handle = self.db.clone_handle();

                task::spawn(async move {
                    let file_key = format!("file_index:{}", path.to_string_lossy());
                    if let Ok(Some(data)) = db_handle.get(file_key.as_bytes()) {
                        if let Ok((file_index, _)) = bincode::decode_from_slice::<FileIndex, _>(
                            &data,
                            bincode::config::standard(),
                        ) {
                            // Check if file needs reindexing
                            let metadata = fs::metadata(&path).ok()?;
                            let current_modified = metadata
                                .modified()
                                .ok()?
                                .duration_since(SystemTime::UNIX_EPOCH)
                                .ok()?
                                .as_secs();

                            if current_modified <= file_index.last_modified {
                                return Some(path);
                            }
                        }
                    }
                    None
                })
            })
            .collect();

        let check_results = futures::future::join_all(file_check_futures).await;
        let indexed_files: HashSet<PathBuf> = check_results
            .into_iter()
            .filter_map(|result| result.ok().flatten())
            .collect();

        // Process files that need indexing
        let files_to_process: Vec<_> = files
            .into_iter()
            .filter(|f| !indexed_files.contains(f))
            .collect();

        // Process files in parallel batches with higher concurrency
        let batch_size = (num_cpus::get() * 2).clamp(8, 32); // More aggressive batching

        // Process multiple batches in parallel for maximum throughput
        let batch_futures: Vec<_> = files_to_process
            .chunks(batch_size)
            .map(|batch| {
                let batch = batch.to_vec();
                let max_size = max_file_size;
                let db_handle = self.db.clone_handle();
                let content_field = self.content_field;
                let path_field = self.path_field;
                let start_line_field = self.start_line_field;
                let end_line_field = self.end_line_field;

                task::spawn(async move {
                    let mut batch_docs = Vec::new();
                    let mut batch_chunk_count = 0;

                    // Process all files in this batch concurrently
                    let file_futures: Vec<_> = batch
                        .into_iter()
                        .map(|file_path| async move {
                            Self::process_file(&file_path, max_size).await.ok()
                        })
                        .collect();

                    let file_results = futures::future::join_all(file_futures).await;

                    // Prepare database batch and documents
                    let mut db_batch = db_handle.create_batch();

                    for (chunks, file_index) in file_results.into_iter().flatten() {
                        if !chunks.is_empty() {
                            batch_chunk_count += chunks.len();

                            // Add chunks to database batch
                            for chunk in &chunks {
                                let chunk_key = format!("chunk:{}", chunk.id);
                                let chunk_data =
                                    bincode::encode_to_vec(chunk, bincode::config::standard())?;
                                db_batch.put(chunk_key.as_bytes(), &chunk_data);

                                // Prepare index document
                                let doc = doc!(
                                    content_field => chunk.content.clone(),
                                    path_field => chunk.file_path.to_string_lossy().to_string(),
                                    start_line_field => chunk.start_line as u64,
                                    end_line_field => chunk.end_line as u64,
                                );
                                batch_docs.push(doc);
                            }

                            // Add file index to batch
                            let file_key =
                                format!("file_index:{}", file_index.file_path.to_string_lossy());
                            let file_data =
                                bincode::encode_to_vec(&file_index, bincode::config::standard())?;
                            db_batch.put(file_key.as_bytes(), &file_data);
                        }
                    }

                    // Write entire batch to database atomically
                    if batch_chunk_count > 0 {
                        db_handle.put_batch(db_batch)?;
                    }

                    Ok::<(Vec<TantivyDocument>, usize), anyhow::Error>((
                        batch_docs,
                        batch_chunk_count,
                    ))
                })
            })
            .collect();

        // Wait for all batches to complete and collect results
        let batch_results = futures::future::join_all(batch_futures).await;

        for (docs, chunk_count) in batch_results.into_iter().flatten().flatten() {
            total_chunks += chunk_count;

            // Add documents to index in bulk
            for doc in docs {
                self.index_writer.add_document(doc)?;
            }
        }

        // Commit index changes
        self.index_writer.commit()?;

        Ok(total_chunks)
    }

    async fn process_file(file_path: &Path, max_file_size: u64) -> Result<(Vec<Chunk>, FileIndex)> {
        // Check file size
        let metadata = fs::metadata(file_path)?;
        if metadata.len() > max_file_size {
            return Ok((
                Vec::new(),
                FileIndex {
                    file_path: file_path.to_owned(),
                    hash: String::new(),
                    last_modified: 0,
                    chunk_count: 0,
                    indexed_at: 0,
                },
            ));
        }

        // Read file content with memory mapping for large files
        let content = if metadata.len() > 1_000_000 {
            // 1MB threshold
            // Use memory mapping for large files
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
            // Use regular file reading for small files
            task::spawn_blocking({
                let file_path = file_path.to_owned();
                move || fs::read_to_string(file_path)
            })
            .await??
        };

        // Generate file hash
        let mut hasher = Hasher::new();
        hasher.update(content.as_bytes());
        let file_hash = hasher.finalize().to_hex().to_string();

        // Create chunks
        let chunks = Self::chunk(file_path, &content, &file_hash, &ChunkConfig::default());
        let chunk_count = chunks.len();

        // Create file index
        let file_index = FileIndex {
            file_path: file_path.to_owned(),
            hash: file_hash,
            last_modified: metadata
                .modified()?
                .duration_since(SystemTime::UNIX_EPOCH)?
                .as_secs(),
            chunk_count,
            indexed_at: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)?
                .as_secs(),
        };

        Ok((chunks, file_index))
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

            // Find a safe UTF-8 boundary near our target
            let mut actual_end = target_end;
            while actual_end > start_byte && !content.is_char_boundary(actual_end) {
                actual_end -= 1;
            }

            // Try to break at a newline if possible
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

            // Extract chunk content safely
            let chunk_content = match content.get(start_byte..actual_end) {
                Some(slice) => slice,
                None => break, // Safety check
            };

            // Calculate line numbers
            let start_line = content[..start_byte].matches('\n').count() + 1;
            let end_line = start_line + chunk_content.matches('\n').count();

            // Create chunk hash
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

            // Move forward with overlap, ensuring UTF-8 boundaries
            let overlap = chunk_config.overlap_size.min((actual_end - start_byte) / 2);
            let next_start = actual_end.saturating_sub(overlap);

            // Find the nearest UTF-8 boundary
            let mut safe_next_start = next_start;
            while safe_next_start > start_byte && !content.is_char_boundary(safe_next_start) {
                safe_next_start -= 1;
            }

            // Ensure we make progress
            if safe_next_start <= start_byte {
                start_byte = actual_end;
            } else {
                start_byte = safe_next_start;
            }
        }

        chunks
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<(Chunk, f32)>> {
        // Only search if query starts with '
        let query = query.trim();
        if !query.starts_with('\'') {
            return Ok(Vec::new());
        }

        // Remove the ' prefix
        let query = &query[1..];
        if query.is_empty() {
            return Ok(Vec::new());
        }

        let searcher = self.index_reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![self.content_field]);

        let query = query_parser.parse_query(query)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))?;

        let mut results = Vec::new();

        for (score, doc_address) in top_docs {
            let retrieved_doc: TantivyDocument = searcher.doc(doc_address)?;

            if let Some(path_value) = retrieved_doc.get_first(self.path_field) {
                if let Some(start_line_value) = retrieved_doc.get_first(self.start_line_field) {
                    if let Some(end_line_value) = retrieved_doc.get_first(self.end_line_field) {
                        let owned_path = OwnedValue::from(path_value);
                        let owned_start = OwnedValue::from(start_line_value);
                        let owned_end = OwnedValue::from(end_line_value);

                        let path_str = match owned_path {
                            OwnedValue::Str(s) => s,
                            _ => String::new(),
                        };
                        let start_line = match owned_start {
                            OwnedValue::U64(n) => n as usize,
                            _ => 0,
                        };
                        let end_line = match owned_end {
                            OwnedValue::U64(n) => n as usize,
                            _ => 0,
                        };

                        // Try to find the corresponding chunk in the database
                        // This is a simplified approach - in production you'd want a more efficient lookup
                        let file_path = PathBuf::from(path_str);
                        if let Ok(content) = fs::read_to_string(&file_path) {
                            let mut hasher = Hasher::new();
                            hasher.update(content.as_bytes());
                            let file_hash = hasher.finalize().to_hex().to_string();

                            // Try to find matching chunk
                            for chunk_id in 0..100 {
                                // arbitrary limit
                                let chunk_key = format!("chunk:{}:{}", file_hash, chunk_id);
                                if let Ok(Some(chunk_data)) = self.db.get(chunk_key.as_bytes()) {
                                    if let Ok((chunk, _)) = bincode::decode_from_slice::<Chunk, _>(
                                        &chunk_data,
                                        bincode::config::standard(),
                                    ) {
                                        if chunk.start_line == start_line
                                            && chunk.end_line == end_line
                                        {
                                            results.push((chunk, score));
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(results)
    }

    pub async fn close(mut self) {
        // Commit any pending changes
        let _ = self.index_writer.commit();
    }
}
