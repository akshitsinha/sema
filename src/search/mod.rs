use crate::types::TextChunk;
use anyhow::{Context, Result};
use parking_lot::RwLock;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tantivy::{
    Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument,
    collector::TopDocs,
    doc,
    query::QueryParser,
    schema::{OwnedValue, STORED, Schema, TEXT},
};

/// Search result containing a chunk and its relevance score
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub chunk: TextChunk,
    pub score: f32,
    pub snippet: String,
    pub highlighted_content: String,
}

/// Search service for full-text search using Tantivy
pub struct SearchService {
    reader: IndexReader,
    writer: Arc<RwLock<IndexWriter>>,
    query_parser: QueryParser,
    schema: Schema,
}

impl SearchService {
    /// Create a new search service
    pub async fn new(config_dir: &Path) -> Result<Self> {
        tokio::fs::create_dir_all(config_dir)
            .await
            .context("Failed to create config directory")?;

        let index_path = config_dir.join("search_index");

        // Define schema
        let mut schema_builder = Schema::builder();
        let _file_path_field = schema_builder.add_text_field("file_path", STORED);
        let content_field = schema_builder.add_text_field("content", TEXT | STORED);
        let _chunk_index_field = schema_builder.add_u64_field("chunk_index", STORED);
        let _start_line_field = schema_builder.add_u64_field("start_line", STORED);
        let _end_line_field = schema_builder.add_u64_field("end_line", STORED);
        let _file_hash_field = schema_builder.add_text_field("file_hash", STORED);

        let schema = schema_builder.build();

        // Create or open index
        let index = if index_path.exists() {
            Index::open_in_dir(&index_path).context("Failed to open existing search index")?
        } else {
            tokio::fs::create_dir_all(&index_path).await?;
            Index::create_in_dir(&index_path, schema.clone())
                .context("Failed to create search index")?
        };

        // Create reader with auto-reload
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .context("Failed to create index reader")?;

        // Create writer
        let writer = index
            .writer(50_000_000) // 50MB heap
            .context("Failed to create index writer")?;

        // Create query parser for content field
        let query_parser = QueryParser::for_index(&index, vec![content_field]);

        Ok(Self {
            reader,
            writer: Arc::new(RwLock::new(writer)),
            query_parser,
            schema,
        })
    }

    /// Index chunks for search
    pub async fn index_chunks(&self, chunks: &[TextChunk]) -> Result<()> {
        let mut writer = self.writer.write();

        let file_path_field = self.schema.get_field("file_path").unwrap();
        let content_field = self.schema.get_field("content").unwrap();
        let chunk_index_field = self.schema.get_field("chunk_index").unwrap();
        let start_line_field = self.schema.get_field("start_line").unwrap();
        let end_line_field = self.schema.get_field("end_line").unwrap();
        let file_hash_field = self.schema.get_field("file_hash").unwrap();

        for chunk in chunks {
            let doc = doc!(
                file_path_field => chunk.file_path.to_string_lossy().as_ref(),
                content_field => chunk.content.as_str(),
                chunk_index_field => chunk.chunk_index as u64,
                start_line_field => chunk.start_line as u64,
                end_line_field => chunk.end_line as u64,
                file_hash_field => chunk.file_hash.as_str()
            );

            writer.add_document(doc)?;
        }

        writer
            .commit()
            .context("Failed to commit to search index")?;
        Ok(())
    }

    /// Remove chunks for specific files from the index
    pub async fn remove_chunks_for_files(&self, file_paths: &[String]) -> Result<()> {
        let mut writer = self.writer.write();

        for file_path in file_paths {
            let term = tantivy::Term::from_field_text(
                self.schema.get_field("file_path").unwrap(),
                file_path,
            );
            writer.delete_term(term);
        }

        writer
            .commit()
            .context("Failed to commit deletions to search index")?;
        Ok(())
    }

    /// Search for chunks matching the query
    pub async fn search(&self, query_str: &str, limit: usize) -> Result<Vec<SearchResult>> {
        if query_str.trim().is_empty() {
            return Ok(Vec::new());
        }

        let searcher = self.reader.searcher();

        let query = self
            .query_parser
            .parse_query(query_str)
            .context("Failed to parse search query")?;

        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(limit))
            .context("Failed to execute search")?;

        let mut results = Vec::new();

        let file_path_field = self.schema.get_field("file_path").unwrap();
        let content_field = self.schema.get_field("content").unwrap();
        let chunk_index_field = self.schema.get_field("chunk_index").unwrap();
        let start_line_field = self.schema.get_field("start_line").unwrap();
        let end_line_field = self.schema.get_field("end_line").unwrap();
        let file_hash_field = self.schema.get_field("file_hash").unwrap();

        for (score, doc_address) in top_docs {
            let retrieved_doc: TantivyDocument = searcher.doc(doc_address)?;

            let file_path = retrieved_doc
                .get_first(file_path_field)
                .and_then(|v| match v {
                    OwnedValue::Str(s) => Some(s.as_str()),
                    _ => None,
                })
                .unwrap_or("")
                .to_string();

            let content = retrieved_doc
                .get_first(content_field)
                .and_then(|v| match v {
                    OwnedValue::Str(s) => Some(s.as_str()),
                    _ => None,
                })
                .unwrap_or("")
                .to_string();

            let chunk_index = retrieved_doc
                .get_first(chunk_index_field)
                .and_then(|v| match v {
                    OwnedValue::U64(n) => Some(*n),
                    _ => None,
                })
                .unwrap_or(0) as usize;

            let start_line = retrieved_doc
                .get_first(start_line_field)
                .and_then(|v| match v {
                    OwnedValue::U64(n) => Some(*n),
                    _ => None,
                })
                .unwrap_or(0) as usize;

            let end_line = retrieved_doc
                .get_first(end_line_field)
                .and_then(|v| match v {
                    OwnedValue::U64(n) => Some(*n),
                    _ => None,
                })
                .unwrap_or(0) as usize;

            let file_hash = retrieved_doc
                .get_first(file_hash_field)
                .and_then(|v| match v {
                    OwnedValue::Str(s) => Some(s.as_str()),
                    _ => None,
                })
                .unwrap_or("")
                .to_string();

            let chunk = TextChunk {
                id: None,
                file_path: PathBuf::from(file_path),
                chunk_index,
                content: content.clone(),
                start_line,
                end_line,
                file_hash,
            };

            // Create snippet and highlighted content
            let snippet = self.create_snippet(&content, query_str, 200);
            let highlighted_content = self.highlight_content(&content, query_str);

            results.push(SearchResult {
                chunk,
                score,
                snippet,
                highlighted_content,
            });
        }

        Ok(results)
    }

    /// Clear all indexed content
    pub async fn clear_index(&self) -> Result<()> {
        let mut writer = self.writer.write();
        writer.delete_all_documents()?;
        writer.commit().context("Failed to clear search index")?;
        Ok(())
    }

    /// Create a snippet around search terms
    fn create_snippet(&self, content: &str, query: &str, max_length: usize) -> String {
        let query_terms: Vec<&str> = query.split_whitespace().collect();

        // Find the first occurrence of any query term
        let mut best_pos = 0;
        for term in &query_terms {
            if let Some(pos) = content.to_lowercase().find(&term.to_lowercase()) {
                best_pos = pos;
                break;
            }
        }

        // Calculate snippet boundaries with proper UTF-8 character boundaries
        let char_indices: Vec<(usize, char)> = content.char_indices().collect();

        if char_indices.is_empty() {
            return String::new();
        }

        // Find character positions instead of byte positions
        let total_chars = char_indices.len();
        let max_chars = max_length / 4; // Rough estimate for character count

        // Find the character index closest to our best_pos
        let best_char_idx = char_indices
            .iter()
            .position(|(byte_pos, _)| *byte_pos >= best_pos)
            .unwrap_or(0);

        let start_char = best_char_idx.saturating_sub(max_chars / 2);
        let end_char = (start_char + max_chars).min(total_chars);

        // Get the actual byte positions
        let start_byte = if start_char == 0 {
            0
        } else {
            char_indices[start_char].0
        };
        let end_byte = if end_char >= total_chars {
            content.len()
        } else {
            char_indices[end_char].0
        };

        let mut snippet = content[start_byte..end_byte].to_string();

        // Add ellipsis if needed
        if start_byte > 0 {
            snippet = format!("...{}", snippet);
        }
        if end_byte < content.len() {
            snippet = format!("{}...", snippet);
        }

        snippet
    }

    /// Highlight search terms in content
    fn highlight_content(&self, content: &str, query: &str) -> String {
        let query_terms: Vec<&str> = query.split_whitespace().collect();
        let mut highlighted = content.to_string();

        for term in query_terms {
            let pattern = regex::Regex::new(&format!(r"(?i)\b{}\b", regex::escape(term)))
                .unwrap_or_else(|_| regex::Regex::new(&regex::escape(term)).unwrap());

            highlighted = pattern
                .replace_all(&highlighted, format!("**{}**", term))
                .to_string();
        }

        highlighted
    }
}
