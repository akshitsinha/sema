use anyhow::Result;
use std::path::Path;
use tantivy::{
    Index, IndexReader, IndexWriter, ReloadPolicy,
    collector::TopDocs,
    directory::MmapDirectory,
    doc,
    query::QueryParser,
    schema::{Field, OwnedValue, STORED, Schema, TEXT},
};

use crate::types::Chunk;

pub struct TextIndexer {
    index: Index,
    writer: IndexWriter,
    reader: IndexReader,
    content_field: Field,
    path_field: Field,
    start_line_field: Field,
    end_line_field: Field,
    id_field: Field,
}

impl TextIndexer {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let index_path = data_dir.join("index");
        std::fs::create_dir_all(&index_path)?;

        let mut schema_builder = Schema::builder();
        let content_field = schema_builder.add_text_field("content", TEXT | STORED);
        let path_field = schema_builder.add_text_field("path", TEXT | STORED);
        let start_line_field = schema_builder.add_u64_field("start_line", STORED);
        let end_line_field = schema_builder.add_u64_field("end_line", STORED);
        let id_field = schema_builder.add_text_field("id", STORED);
        let schema = schema_builder.build();

        let index_dir = MmapDirectory::open(&index_path)?;
        let index = Index::open_or_create(index_dir, schema)?;
        let writer = index.writer(200_000_000)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;

        Ok(Self {
            index,
            writer,
            reader,
            content_field,
            path_field,
            start_line_field,
            end_line_field,
            id_field,
        })
    }

    pub fn index_chunks(&mut self, chunks: &[Chunk]) -> Result<()> {
        for chunk in chunks {
            let doc = doc!(
                self.content_field => chunk.content.clone(),
                self.path_field => chunk.file_path.to_string_lossy().to_string(),
                self.start_line_field => chunk.start_line as u64,
                self.end_line_field => chunk.end_line as u64,
                self.id_field => chunk.id.clone(),
            );
            self.writer.add_document(doc)?;
        }

        self.writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<(Chunk, f32)>> {
        if query.is_empty() {
            return Ok(Vec::new());
        }

        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![self.content_field]);
        let parsed_query = query_parser.parse_query(query)?;
        let top_docs = searcher.search(&parsed_query, &TopDocs::with_limit(limit))?;

        let mut results = Vec::new();
        for (score, doc_address) in top_docs {
            let doc = searcher.doc::<tantivy::TantivyDocument>(doc_address)?;

            let id = doc
                .get_first(self.id_field)
                .map(|v| OwnedValue::from(v))
                .and_then(|v| {
                    if let OwnedValue::Str(s) = v {
                        Some(s)
                    } else {
                        None
                    }
                })
                .unwrap_or_default();
            let path_str = doc
                .get_first(self.path_field)
                .map(|v| OwnedValue::from(v))
                .and_then(|v| {
                    if let OwnedValue::Str(s) = v {
                        Some(s)
                    } else {
                        None
                    }
                })
                .unwrap_or_default();
            let content = doc
                .get_first(self.content_field)
                .map(|v| OwnedValue::from(v))
                .and_then(|v| {
                    if let OwnedValue::Str(s) = v {
                        Some(s)
                    } else {
                        None
                    }
                })
                .unwrap_or_default();
            let start_line = doc
                .get_first(self.start_line_field)
                .map(|v| OwnedValue::from(v))
                .and_then(|v| {
                    if let OwnedValue::U64(n) = v {
                        Some(n)
                    } else {
                        None
                    }
                })
                .unwrap_or(0) as usize;
            let end_line = doc
                .get_first(self.end_line_field)
                .map(|v| OwnedValue::from(v))
                .and_then(|v| {
                    if let OwnedValue::U64(n) = v {
                        Some(n)
                    } else {
                        None
                    }
                })
                .unwrap_or(0) as usize;

            results.push((
                Chunk {
                    id,
                    file_path: std::path::PathBuf::from(path_str),
                    start_line,
                    end_line,
                    content,
                },
                score,
            ));
        }

        Ok(results)
    }

    pub fn commit(&mut self) -> Result<()> {
        self.writer.commit()?;
        Ok(())
    }
}
