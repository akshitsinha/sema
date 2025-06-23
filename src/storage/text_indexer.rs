use anyhow::Result;
use std::path::Path;
use tantivy::{
    Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument,
    collector::TopDocs,
    directory::MmapDirectory,
    doc,
    query::QueryParser,
    schema::{Field, OwnedValue, STORED, Schema, TEXT},
};

use crate::types::Chunk;

/// Tantivy indexer for full-text search of chunks
pub struct TextIndexer {
    index: Index,
    index_writer: IndexWriter,
    index_reader: IndexReader,
    content_field: Field,
    path_field: Field,
    start_line_field: Field,
    end_line_field: Field,
    id_field: Field,
    hash_field: Field,
}

impl TextIndexer {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let index_path = data_dir.join("index");
        std::fs::create_dir_all(&index_path)?;

        // Initialize Tantivy schema
        let mut schema_builder = Schema::builder();
        let content_field = schema_builder.add_text_field("content", TEXT | STORED);
        let path_field = schema_builder.add_text_field("path", TEXT | STORED);
        let start_line_field = schema_builder.add_u64_field("start_line", STORED);
        let end_line_field = schema_builder.add_u64_field("end_line", STORED);
        let id_field = schema_builder.add_text_field("id", STORED);
        let hash_field = schema_builder.add_text_field("hash", STORED);
        let schema = schema_builder.build();

        // Initialize Tantivy index
        let index_dir = MmapDirectory::open(&index_path)?;
        let index = Index::open_or_create(index_dir, schema.clone())?;

        let index_writer = index.writer(200_000_000)?; // 200MB heap
        let index_reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;

        Ok(Self {
            index,
            index_writer,
            index_reader,
            content_field,
            path_field,
            start_line_field,
            end_line_field,
            id_field,
            hash_field,
        })
    }

    pub fn index_chunks(&mut self, chunks: &[Chunk]) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }

        // Add documents to index in bulk
        for chunk in chunks {
            let doc = doc!(
                self.content_field => chunk.content.clone(),
                self.path_field => chunk.file_path.to_string_lossy().to_string(),
                self.start_line_field => chunk.start_line as u64,
                self.end_line_field => chunk.end_line as u64,
                self.id_field => chunk.id.clone(),
                self.hash_field => chunk.hash.clone(),
            );
            self.index_writer.add_document(doc)?;
        }

        // Commit changes
        self.index_writer.commit()?;

        // Reload the reader to see new documents
        self.index_reader.reload()?;

        Ok(())
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<(Chunk, f32)>> {
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

            if let (
                Some(id_value),
                Some(path_value),
                Some(start_line_value),
                Some(end_line_value),
                Some(content_value),
                Some(hash_value),
            ) = (
                retrieved_doc.get_first(self.id_field),
                retrieved_doc.get_first(self.path_field),
                retrieved_doc.get_first(self.start_line_field),
                retrieved_doc.get_first(self.end_line_field),
                retrieved_doc.get_first(self.content_field),
                retrieved_doc.get_first(self.hash_field),
            ) {
                let id = match OwnedValue::from(id_value) {
                    OwnedValue::Str(s) => s,
                    _ => continue,
                };
                let path_str = match OwnedValue::from(path_value) {
                    OwnedValue::Str(s) => s,
                    _ => continue,
                };
                let start_line = match OwnedValue::from(start_line_value) {
                    OwnedValue::U64(n) => n as usize,
                    _ => continue,
                };
                let end_line = match OwnedValue::from(end_line_value) {
                    OwnedValue::U64(n) => n as usize,
                    _ => continue,
                };
                let content = match OwnedValue::from(content_value) {
                    OwnedValue::Str(s) => s,
                    _ => continue,
                };
                let hash = match OwnedValue::from(hash_value) {
                    OwnedValue::Str(s) => s,
                    _ => continue,
                };

                let chunk = Chunk {
                    id,
                    file_path: std::path::PathBuf::from(path_str),
                    start_line,
                    end_line,
                    content,
                    hash,
                };

                results.push((chunk, score));
            }
        }

        Ok(results)
    }

    pub fn commit(&mut self) -> Result<()> {
        self.index_writer.commit()?;
        Ok(())
    }
}
