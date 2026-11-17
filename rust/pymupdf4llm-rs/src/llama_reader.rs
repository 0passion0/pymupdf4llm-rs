use std::collections::BTreeMap;
use std::path::Path;
use serde::{Deserialize, Serialize};

use crate::config::ToMarkdownOptions;
use crate::error::Result;
use crate::{to_markdown, MarkdownOutput};

#[cfg(feature = "async")]
use crate::to_markdown_async;

/// Compatible replica of `pymupdf4llm.LlamaMarkdownReader` for RAG / LlamaIndex pipelines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlamaDocument {
    pub id_: String,
    pub text: String,
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default)]
pub struct LlamaMarkdownReader {
    pub options: ToMarkdownOptions,
}

impl LlamaMarkdownReader {
    pub fn new(options: ToMarkdownOptions) -> Self {
        Self { options }
    }

    pub fn load_data(&self, path: impl AsRef<Path>) -> Result<Vec<LlamaDocument>> {
        let mut opts = self.options.clone();
        opts.page_chunks = true;
        let MarkdownOutput::Chunks(chunks) = to_markdown(path.as_ref(), opts)? else {
            unreachable!();
        };
        Ok(Self::chunks_to_llama_docs(chunks))
    }

    #[cfg(feature = "async")]
    pub async fn load_data_async(&self, path: impl AsRef<Path>) -> Result<Vec<LlamaDocument>> {
        let mut opts = self.options.clone();
        opts.page_chunks = true;
        let MarkdownOutput::Chunks(chunks) = to_markdown_async(path.as_ref(), opts).await? else {
            unreachable!();
        };
        Ok(Self::chunks_to_llama_docs(chunks))
    }

    fn chunks_to_llama_docs(chunks: Vec<crate::PageChunk>) -> Vec<LlamaDocument> {
        chunks
            .into_iter()
            .map(|c| {
                let mut metadata = BTreeMap::new();
                metadata.insert("file_path".into(), serde_json::json!(c.metadata.file_path));
                metadata.insert("page".into(), serde_json::json!(c.metadata.page));
                metadata.insert("page_count".into(), serde_json::json!(c.metadata.page_count));
                metadata.insert("title".into(), serde_json::json!(c.metadata.title));
                metadata.insert("author".into(), serde_json::json!(c.metadata.author));
                metadata.insert("toc_items".into(), serde_json::json!(c.toc_items));
                metadata.insert("tables".into(), serde_json::json!(c.tables));
                LlamaDocument {
                    id_: format!("{}:{}", c.metadata.file_path, c.metadata.page),
                    text: c.text,
                    metadata,
                }
            })
            .collect()
    }
}
