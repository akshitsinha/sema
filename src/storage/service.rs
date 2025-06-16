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

        let index_writer = index.writer(50_000_000)?; // 50MB heap
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

        // Process files in parallel batches for reading, but serialize DB writes
        let batch_size = num_cpus::get().max(4);

        for batch in files_to_process.chunks(batch_size) {
            // Process file data in parallel (reading and chunking)
            let mut file_data_futures = Vec::new();

            for file_path in batch {
                let path = file_path.clone();
                let max_size = max_file_size;

                let future =
                    task::spawn(
                        async move { Self::process_file_data_static(&path, max_size).await },
                    );

                file_data_futures.push(future);
            }

            // Wait for all file processing to complete
            let batch_results = futures::future::join_all(file_data_futures).await;

            // Write results to database in parallel using batched writes
            let write_futures: Vec<_> = batch_results
                .into_iter()
                .filter_map(|task_result| {
                    if let Ok(Ok((chunks, file_index))) = task_result {
                        if !chunks.is_empty() {
                            Some((chunks, file_index))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .map(|(chunks, file_index)| {
                    let db_handle = self.db.clone_handle();
                    let content_field = self.content_field;
                    let path_field = self.path_field;
                    let start_line_field = self.start_line_field;
                    let end_line_field = self.end_line_field;

                    task::spawn(async move {
                        // Create batch for this file
                        let mut batch = db_handle.create_batch();
                        let mut docs = Vec::new();

                        // Add chunks to batch
                        for chunk in &chunks {
                            let chunk_key = format!("chunk:{}", chunk.id);
                            let chunk_data =
                                bincode::encode_to_vec(chunk, bincode::config::standard())?;
                            batch.put(chunk_key.as_bytes(), &chunk_data);

                            let doc = doc!(
                                content_field => chunk.content.clone(),
                                path_field => chunk.file_path.to_string_lossy().to_string(),
                                start_line_field => chunk.start_line as u64,
                                end_line_field => chunk.end_line as u64,
                            );
                            docs.push(doc);
                        }

                        // Add file index to batch
                        let file_key =
                            format!("file_index:{}", file_index.file_path.to_string_lossy());
                        let file_data =
                            bincode::encode_to_vec(&file_index, bincode::config::standard())?;
                        batch.put(file_key.as_bytes(), &file_data);

                        // Write batch to database
                        db_handle.put_batch(batch)?;

                        Ok::<(Vec<tantivy::TantivyDocument>, usize), anyhow::Error>((
                            docs,
                            chunks.len(),
                        ))
                    })
                })
                .collect();

            // Wait for all writes to complete
            let write_results = futures::future::join_all(write_futures).await;

            // Add documents to index in batches for better performance
            let mut all_docs = Vec::new();
            for write_result in write_results {
                if let Ok(Ok((docs, chunk_count))) = write_result {
                    total_chunks += chunk_count;
                    all_docs.extend(docs);
                }
            }

            // Batch add documents to index
            for doc in all_docs {
                self.index_writer.add_document(doc)?;
            }
        }

        // Commit index changes
        self.index_writer.commit()?;

        Ok(total_chunks)
    }

    async fn process_file_data_static(
        file_path: &Path,
        max_file_size: u64,
    ) -> Result<(Vec<Chunk>, FileIndex)> {
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

        // Read file content
        let content = task::spawn_blocking({
            let file_path = file_path.to_owned();
            move || fs::read_to_string(file_path)
        })
        .await??;

        // Generate file hash
        let mut hasher = Hasher::new();
        hasher.update(content.as_bytes());
        let file_hash = hasher.finalize().to_hex().to_string();

        // Create chunks
        let chunks =
            Self::create_chunks_static(file_path, &content, &file_hash, &ChunkConfig::default());
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

    fn create_chunks_static(
        file_path: &Path,
        content: &str,
        file_hash: &str,
        chunk_config: &ChunkConfig,
    ) -> Vec<Chunk> {
        let lines: Vec<&str> = content.lines().collect();
        let mut chunks = Vec::new();
        let mut current_pos = 0;
        let mut chunk_id = 0;

        while current_pos < content.len() {
            let mut chunk_content = String::new();
            let mut start_line = 0;
            let mut end_line = 0;
            let mut current_line = 0;
            let mut char_count = 0;

            // Find which line we're starting from
            let mut line_start_pos = 0;
            for (line_idx, line) in lines.iter().enumerate() {
                if line_start_pos + line.len() + 1 > current_pos {
                    start_line = line_idx + 1;
                    current_line = line_idx;
                    break;
                }
                line_start_pos += line.len() + 1;
            }

            // Build chunk content
            while current_line < lines.len() && char_count < chunk_config.chunk_size {
                if !chunk_content.is_empty() {
                    chunk_content.push('\n');
                    char_count += 1;
                }

                let line = lines[current_line];
                chunk_content.push_str(line);
                char_count += line.len();
                end_line = current_line + 1;
                current_line += 1;
            }

            // Skip if chunk is too small
            if chunk_content.len() < chunk_config.min_chunk_size {
                break;
            }

            // Create chunk
            let chunk = Chunk {
                id: format!("{}:{}", file_hash, chunk_id),
                file_path: file_path.to_owned(),
                start_line,
                end_line,
                content: chunk_content.clone(),
                hash: {
                    let mut hasher = Hasher::new();
                    hasher.update(chunk_content.as_bytes());
                    hasher.finalize().to_hex().to_string()
                },
            };

            chunks.push(chunk);
            chunk_id += 1;

            // Move position forward, accounting for overlap
            let overlap_chars = chunk_config.overlap_size.min(chunk_content.len() / 2);
            current_pos += chunk_content.len() - overlap_chars;
        }

        chunks
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<(Chunk, f32)>> {
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
                        let path_str = match path_value {
                            OwnedValue::Str(s) => s,
                            _ => "",
                        };
                        let start_line = match start_line_value {
                            OwnedValue::U64(n) => *n as usize,
                            _ => 0,
                        };
                        let end_line = match end_line_value {
                            OwnedValue::U64(n) => *n as usize,
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
