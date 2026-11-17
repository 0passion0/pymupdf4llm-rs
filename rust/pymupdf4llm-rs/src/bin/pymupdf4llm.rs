use std::path::PathBuf;
use clap::{Parser, ValueEnum};

use pymupdf4llm_rs::{
    batch_to_markdown_async, to_markdown_async, ImageFormat, MarkdownOutput, TableStrategy,
    ToMarkdownOptions,
};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliTableStrategy {
    LinesStrict,
    Lines,
    Text,
    Explicit,
}

impl From<CliTableStrategy> for TableStrategy {
    fn from(v: CliTableStrategy) -> Self {
        match v {
            CliTableStrategy::LinesStrict => TableStrategy::LinesStrict,
            CliTableStrategy::Lines => TableStrategy::Lines,
            CliTableStrategy::Text => TableStrategy::Text,
            CliTableStrategy::Explicit => TableStrategy::Explicit,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "pymupdf4llm", about = "Pure-Rust PDF to Markdown extractor compatible with pymupdf4llm")]
struct Cli {
    /// Input PDF file(s)
    #[arg(required = true)]
    inputs: Vec<PathBuf>,

    /// 0-indexed comma-separated page numbers
    #[arg(long, value_delimiter = ',')]
    pages: Option<Vec<usize>>,

    /// Return JSON page chunks instead of single Markdown string
    #[arg(long)]
    page_chunks: bool,

    /// Insert page separator lines (`-----`)
    #[arg(long)]
    page_separators: bool,

    /// Table extraction strategy
    #[arg(long, value_enum, default_value_t = CliTableStrategy::LinesStrict)]
    table_strategy: CliTableStrategy,

    /// Save extracted images to disk
    #[arg(long)]
    write_images: bool,

    /// Embed images as base64 data URIs
    #[arg(long)]
    embed_images: bool,

    /// Directory for written images
    #[arg(long, default_value = "")]
    image_path: PathBuf,

    /// Disable code block formatting for monospace text
    #[arg(long)]
    ignore_code: bool,

    /// Include extracted word bounding boxes in page chunks
    #[arg(long)]
    extract_words: bool,

    /// Maximum concurrent page/document workers
    #[arg(long, default_value_t = 8)]
    concurrency: usize,

    /// Optional output file path (defaults to stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow_or_result::Result<()> {
    let cli = Cli::parse();
    let mut builder = ToMarkdownOptions::builder()
        .page_chunks(cli.page_chunks)
        .page_separators(cli.page_separators)
        .table_strategy(cli.table_strategy.into())
        .write_images(cli.write_images)
        .embed_images(cli.embed_images)
        .image_path(cli.image_path)
        .image_format(ImageFormat::Png)
        .ignore_code(cli.ignore_code)
        .extract_words(cli.extract_words)
        .max_page_concurrency(cli.concurrency)
        .max_doc_concurrency(cli.concurrency);

    if let Some(pages) = cli.pages {
        builder = builder.pages(pages);
    }

    let opts = builder.build();
    let rendered = if cli.inputs.len() == 1 {
        let out = to_markdown_async(&cli.inputs[0], opts).await?;
        format_output(out)
    } else {
        let results = batch_to_markdown_async(cli.inputs, opts).await;
        serde_json::to_string_pretty(
            &results
                .into_iter()
                .map(|r| {
                    serde_json::json!({
                        "path": r.path,
                        "elapsed_ms": r.elapsed_ms,
                        "result": r.output.ok()
                    })
                })
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default()
    };

    if let Some(out_path) = cli.output {
        tokio::fs::write(out_path, rendered).await?;
    } else {
        println!("{rendered}");
    }
    Ok(())
}

mod anyhow_or_result {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
}

fn format_output(out: MarkdownOutput) -> String {
    match out {
        MarkdownOutput::Text(s) => s,
        MarkdownOutput::Chunks(chunks) => serde_json::to_string_pretty(&chunks).unwrap_or_default(),
    }
}
