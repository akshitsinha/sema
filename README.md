# Sema

A semantic search tool for codebases that uses AI embeddings to find relevant code snippets across your projects.

## What is Sema?

Sema is a terminal-based semantic search engine for source code. Unlike traditional text-based search tools like `grep` or `ripgrep`, Sema understands the meaning and context of your code, allowing you to search for functionality rather than just exact text matches.

## Features

- **Semantic Search**: Find code by meaning, not just keywords
- **Interactive TUI**: Clean terminal interface for browsing results
- **File Preview**: View full files with automatic positioning at relevant sections
- **Fast Indexing**: Efficient crawling and chunking of codebases
- **Vector Storage**: Uses Lance for high-performance vector search

## Installation

```bash
# Clone the repository
git clone <repository-url>
cd sema

# Build the project
cargo build --release

# Install locally
cargo install --path .
```

## Usage

```bash
# Search in current directory
sema

# Search in specific directory
sema /path/to/your/codebase
```

### Navigation

- **Search**: Type your query and press Enter
- **Browse Results**: Use ↑/↓ arrow keys to navigate search results
- **Preview Files**: Press Enter to open file preview
- **Scroll Preview**: Use ↑/↓ in preview mode to scroll through files
- **Return to Search**: Press Esc to return to search input
- **Quit**: Press Ctrl+C or q to exit

## How It Works

1. **Crawling**: Discovers all source files in your directory
2. **Chunking**: Breaks files into meaningful code segments
3. **Embedding**: Generates semantic embeddings for each chunk
4. **Indexing**: Stores embeddings in a vector database
5. **Search**: Finds semantically similar code based on your query

## Configuration

Sema can be configured using the `~/.sema/config.toml` file.

## License

This project is licensed under the MIT License - see the [LICENSE.md](LICENSE.md) file for details.
