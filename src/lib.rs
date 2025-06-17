//! Sema - Semantic File Search
//!
//! A terminal application for crawling text files recursively and providing
//! full-text search capabilities using Tantivy.

pub mod cli;
pub mod config;
pub mod crawler;
pub mod storage;
pub mod tui;
pub mod types;

pub use anyhow::{Error, Result};
