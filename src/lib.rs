//! Sema - Semantic File Search
//!
//! A terminal application for crawling text files recursively and providing
//! semantic search capabilities using local embedding models and Qdrant vector database.

pub mod cli;
pub mod config;
pub mod crawler;
pub mod embeddings;
pub mod storage;
pub mod tui;
pub mod types;
pub mod vector_db;

pub use anyhow::{Error, Result};
