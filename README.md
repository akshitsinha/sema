# Sema

**⚠️ EXPERIMENTAL PROJECT - ACTIVE DEVELOPMENT**

This project is in early development. A lot of things don't work as expected, most notably improving the accuracy and performance of semantic analysis will take time.

## The Problem

Managing large collections of knowledge scattered across your system is challenging. Whether you have research papers, books, documentation, or notes spread across different folders, finding specific information often requires either:

- Manually browsing through directories hoping to locate the right file
- Using basic search tools that only match exact keywords, missing conceptually related content
- Remembering precise wording from documents you read weeks or months ago

Sema addresses this by enabling natural language queries that find semantically relevant content across your entire knowledge base.

## What is Sema?

Sema is a semantic search tool that understands the meaning behind your queries rather than just matching keywords. It allows you to search your personal collection of documents, code, and notes using natural questions and concepts.

## Example Queries

**Literature and books:**

- "What did Aristotle say about friendship?"
- "How do science fiction authors approach time travel?"
- "What strategies did Napoleon use in the Russian campaign?"

**Research and documentation:**

- "Best practices for user authentication"
- "How does overfitting work in machine learning?"
- "What caused the 2008 financial crisis?"

**Technical knowledge:**

- "Database optimization techniques for large datasets"
- "Common microservices communication patterns"

You can ask questions naturally without needing to remember exact phrases or terminology.

## Features

- Natural language search using semantic understanding
- Interactive terminal interface for browsing results
- File preview with automatic positioning at relevant sections
- Fast indexing and search performance
- Support for various text file formats (markdown, code, PDFs TODO, etc.)

## Installation and Usage

Requirements: Rust toolchain

```bash
git clone <repository-url>
cd sema
cargo build --release
cargo install --path .
```

Basic usage:

```bash
# Search current directory
sema

# Search specific directory
sema /path/to/your/content
```

![sema](https://github.com/user-attachments/assets/f9c0bf6b-3d49-49a6-a9d1-64541772821e)

**Navigation:**

- Type your query and press Enter
- Use arrow keys to browse results
- Press Enter to preview files
- Press Esc to return to search
- Press Ctrl+C or 'q' to exit

## How It Works

Sema scans your files, generates semantic embeddings using AI models, and builds a searchable index. When you submit a query, it finds content that matches the conceptual meaning rather than just exact keyword matches.

## Configuration

Settings can be customized in `~/.sema/config.toml`.

## License

MIT License - see [LICENSE.md](LICENSE.md) for details.
