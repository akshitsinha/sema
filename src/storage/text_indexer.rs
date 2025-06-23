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

pub struct TextIndexer {
    index: Index,
    index_writer: IndexWriter,
    index_reader: IndexReader,
    schema_fields: SchemaFields,
}

struct SchemaFields {
    content: Field,
    path: Field,
    start_line: Field,
    end_line: Field,
    id: Field,
}

impl TextIndexer {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let index_path = data_dir.join("index");
        std::fs::create_dir_all(&index_path)?;

        let (schema, fields) = Self::create_schema();
        let index = Self::create_or_open_index(&index_path, schema)?;

        let index_writer = index.writer(200_000_000)?; // 200MB heap
        let index_reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;

        Ok(Self {
            index,
            index_writer,
            index_reader,
            schema_fields: fields,
        })
    }

    fn create_schema() -> (Schema, SchemaFields) {
        let mut schema_builder = Schema::builder();

        let fields = SchemaFields {
            content: schema_builder.add_text_field("content", TEXT | STORED),
            path: schema_builder.add_text_field("path", TEXT | STORED),
            start_line: schema_builder.add_u64_field("start_line", STORED),
            end_line: schema_builder.add_u64_field("end_line", STORED),
            id: schema_builder.add_text_field("id", STORED),
        };

        (schema_builder.build(), fields)
    }

    fn create_or_open_index(index_path: &Path, schema: Schema) -> Result<Index> {
        let index_dir = MmapDirectory::open(index_path)?;
        Ok(Index::open_or_create(index_dir, schema)?)
    }

    pub fn index_chunks(&mut self, chunks: &[Chunk]) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }

        for chunk in chunks {
            let doc = self.create_document(chunk);
            self.index_writer.add_document(doc)?;
        }

        self.commit_and_reload()?;
        Ok(())
    }

    fn create_document(&self, chunk: &Chunk) -> TantivyDocument {
        doc!(
            self.schema_fields.content => chunk.content.clone(),
            self.schema_fields.path => chunk.file_path.to_string_lossy().to_string(),
            self.schema_fields.start_line => chunk.start_line as u64,
            self.schema_fields.end_line => chunk.end_line as u64,
            self.schema_fields.id => chunk.id.clone(),
        )
    }

    fn commit_and_reload(&mut self) -> Result<()> {
        self.index_writer.commit()?;
        self.index_reader.reload()?;
        Ok(())
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<(Chunk, f32)>> {
        if query.is_empty() {
            return Ok(Vec::new());
        }

        let searcher = self.index_reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![self.schema_fields.content]);

        let parsed_query = query_parser.parse_query(query)?;
        let top_docs = searcher.search(&parsed_query, &TopDocs::with_limit(limit))?;

        let mut results = Vec::new();

        for (score, doc_address) in top_docs {
            if let Some(chunk) = self.extract_chunk_from_document(&searcher, doc_address)? {
                results.push((chunk, score));
            }
        }

        Ok(results)
    }

    fn extract_chunk_from_document(
        &self,
        searcher: &tantivy::Searcher,
        doc_address: tantivy::DocAddress,
    ) -> Result<Option<Chunk>> {
        let retrieved_doc: TantivyDocument = searcher.doc(doc_address)?;

        let id = self.extract_string_field(&retrieved_doc, self.schema_fields.id)?;
        let path_str = self.extract_string_field(&retrieved_doc, self.schema_fields.path)?;
        let content = self.extract_string_field(&retrieved_doc, self.schema_fields.content)?;

        let start_line =
            self.extract_u64_field(&retrieved_doc, self.schema_fields.start_line)? as usize;
        let end_line =
            self.extract_u64_field(&retrieved_doc, self.schema_fields.end_line)? as usize;

        if let (Some(id), Some(path_str), Some(content)) = (id, path_str, content) {
            Ok(Some(Chunk {
                id,
                file_path: std::path::PathBuf::from(path_str),
                start_line,
                end_line,
                content,
            }))
        } else {
            Ok(None)
        }
    }

    fn extract_string_field(&self, doc: &TantivyDocument, field: Field) -> Result<Option<String>> {
        if let Some(value) = doc.get_first(field) {
            match OwnedValue::from(value) {
                OwnedValue::Str(s) => Ok(Some(s)),
                _ => Ok(None),
            }
        } else {
            Ok(None)
        }
    }

    fn extract_u64_field(&self, doc: &TantivyDocument, field: Field) -> Result<u64> {
        if let Some(value) = doc.get_first(field) {
            match OwnedValue::from(value) {
                OwnedValue::U64(n) => Ok(n),
                _ => Ok(0),
            }
        } else {
            Ok(0)
        }
    }

    pub fn commit(&mut self) -> Result<()> {
        self.index_writer.commit()?;
        Ok(())
    }
}
