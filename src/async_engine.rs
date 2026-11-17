use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use futures::stream::{self, Stream, StreamExt};
use tokio::sync::Semaphore;

use crate::config::ToMarkdownOptions;
use crate::error::{PdfError, Result};
use crate::layout::{analyze_page_layout, LayoutPage};
use crate::markdown::chunk::PageChunk;
use crate::markdown::render_page_chunk;
use crate::pdf::{ParsedPage, PdfDocument};
use crate::{
    build_header_resolver, process_single_page, resolve_target_pages,
    strip_running_headers_from_pages, MarkdownOutput,
};

#[derive(Debug)]
pub struct BatchItemResult {
    pub path: PathBuf,
    pub output: Result<MarkdownOutput>,
    pub elapsed_ms: f64,
}

/// Process-wide reusable extraction engine for Web servers (Axum / Actix / Tonic).
/// Caps total in-flight PDF documents and concurrent page workers globally to avoid
/// thread exhaustion and OOM spikes under burst HTTP traffic without affecting output quality.
pub struct PdfEngine {
    doc_semaphore: Arc<Semaphore>,
    page_semaphore: Arc<Semaphore>,
}

impl PdfEngine {
    pub fn new(max_concurrent_docs: usize, max_concurrent_pages: usize) -> Self {
        Self {
            doc_semaphore: Arc::new(Semaphore::new(max_concurrent_docs.max(1))),
            page_semaphore: Arc::new(Semaphore::new(max_concurrent_pages.max(1))),
        }
    }

    pub fn global() -> &'static Arc<Self> {
        static GLOBAL: OnceLock<Arc<PdfEngine>> = OnceLock::new();
        GLOBAL.get_or_init(|| {
            let cpus = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);
            Arc::new(Self::new(cpus.max(4), (cpus * 2).max(8)))
        })
    }

    pub fn default_options() -> &'static Arc<ToMarkdownOptions> {
        static DEFAULT: OnceLock<Arc<ToMarkdownOptions>> = OnceLock::new();
        DEFAULT.get_or_init(|| Arc::new(ToMarkdownOptions::default()))
    }

    pub async fn extract_bytes(
        &self,
        bytes: Vec<u8>,
        options: ToMarkdownOptions,
    ) -> Result<MarkdownOutput> {
        self.extract_bytes_shared(bytes, Arc::new(options)).await
    }

    pub async fn extract_bytes_shared(
        &self,
        bytes: Vec<u8>,
        options: Arc<ToMarkdownOptions>,
    ) -> Result<MarkdownOutput> {
        let _doc_permit = self
            .doc_semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| PdfError::TaskJoin(e.to_string()))?;
        let doc = tokio::task::spawn_blocking(move || PdfDocument::from_bytes(&bytes))
            .await
            .map_err(|e| PdfError::TaskJoin(e.to_string()))??;
        to_markdown_doc_with_semaphore(Arc::new(doc), options, Arc::clone(&self.page_semaphore))
            .await
    }

    pub async fn extract_doc_shared(
        &self,
        doc: Arc<PdfDocument>,
        options: Arc<ToMarkdownOptions>,
    ) -> Result<MarkdownOutput> {
        let _doc_permit = self
            .doc_semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| PdfError::TaskJoin(e.to_string()))?;
        to_markdown_doc_with_semaphore(doc, options, Arc::clone(&self.page_semaphore)).await
    }

    pub async fn extract_path(
        &self,
        path: impl AsRef<Path>,
        options: ToMarkdownOptions,
    ) -> Result<MarkdownOutput> {
        let _doc_permit = self
            .doc_semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| PdfError::TaskJoin(e.to_string()))?;
        let path_buf = path.as_ref().to_path_buf();
        let bytes = tokio::fs::read(&path_buf).await?;
        let mut doc = tokio::task::spawn_blocking(move || PdfDocument::from_bytes(&bytes))
            .await
            .map_err(|e| PdfError::TaskJoin(e.to_string()))??;
        doc.file_path = Some(path_buf);
        to_markdown_doc_with_semaphore(
            Arc::new(doc),
            Arc::new(options),
            Arc::clone(&self.page_semaphore),
        )
        .await
    }
}

/// Asynchronous high-concurrency `to_markdown_async` for a file path.
/// Uses `tokio::fs` for non-blocking I/O, bounded `Semaphore` backpressure, and
/// `tokio::task::spawn_blocking` offloaded onto Rayon/Tokio blocking pools.
pub async fn to_markdown_async(
    path: impl AsRef<Path>,
    options: ToMarkdownOptions,
) -> Result<MarkdownOutput> {
    let path_buf = path.as_ref().to_path_buf();
    let bytes = tokio::fs::read(&path_buf).await?;
    let mut doc = tokio::task::spawn_blocking(move || PdfDocument::from_bytes(&bytes))
        .await
        .map_err(|e| PdfError::TaskJoin(e.to_string()))??;
    doc.file_path = Some(path_buf);
    to_markdown_doc_async(Arc::new(doc), options).await
}

/// Asynchronous `to_markdown_bytes_async` for in-memory PDF payloads.
pub async fn to_markdown_bytes_async(
    bytes: Vec<u8>,
    options: ToMarkdownOptions,
) -> Result<MarkdownOutput> {
    let doc = tokio::task::spawn_blocking(move || PdfDocument::from_bytes(&bytes))
        .await
        .map_err(|e| PdfError::TaskJoin(e.to_string()))??;
    to_markdown_doc_async(Arc::new(doc), options).await
}

/// Concurrent page-level pipeline sharing `Arc<PdfDocument>` and `Arc<dyn HeaderResolver>` zero-copy.
pub async fn to_markdown_doc_async(
    doc: Arc<PdfDocument>,
    options: ToMarkdownOptions,
) -> Result<MarkdownOutput> {
    let semaphore = Arc::new(Semaphore::new(options.concurrency.max_page_concurrency));
    to_markdown_doc_with_semaphore(doc, Arc::new(options), semaphore).await
}

async fn to_markdown_doc_with_semaphore(
    doc: Arc<PdfDocument>,
    opts_arc: Arc<ToMarkdownOptions>,
    semaphore: Arc<Semaphore>,
) -> Result<MarkdownOutput> {
    let target_pages = resolve_target_pages(&doc, &opts_arc)?;

    let header_resolver = {
        let doc_c = Arc::clone(&doc);
        let opts_c = Arc::clone(&opts_arc);
        tokio::task::spawn_blocking(move || build_header_resolver(&doc_c, &opts_c))
            .await
            .map_err(|e| PdfError::TaskJoin(e.to_string()))??
    };

    let total_pages = target_pages.len();
    let completed = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let mut chunks = if opts_arc.hygiene.dedup_running_headers && total_pages >= 3 {
        let mut handles = Vec::with_capacity(total_pages);
        for page_idx in target_pages {
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| PdfError::TaskJoin(e.to_string()))?;
            let doc_c = Arc::clone(&doc);
            let opts_c = Arc::clone(&opts_arc);
            handles.push(tokio::task::spawn_blocking(move || {
                let _guard = permit;
                let parsed = doc_c.parse_page(page_idx, &opts_c)?;
                let layout = analyze_page_layout(&parsed, &opts_c);
                Ok::<(ParsedPage, LayoutPage), PdfError>((parsed, layout))
            }));
        }
        let mut analyzed = Vec::with_capacity(total_pages);
        for h in handles {
            analyzed.push(h.await.map_err(|e| PdfError::TaskJoin(e.to_string()))??);
        }
        strip_running_headers_from_pages(&mut analyzed, &opts_arc);

        let mut render_handles = Vec::with_capacity(analyzed.len());
        for (parsed, layout) in analyzed {
            let doc_c = Arc::clone(&doc);
            let opts_c = Arc::clone(&opts_arc);
            let hdr_c = Arc::clone(&header_resolver);
            let comp_c = Arc::clone(&completed);
            render_handles.push(tokio::task::spawn_blocking(move || {
                let chunk = render_page_chunk(&doc_c, &parsed, &layout, &opts_c, hdr_c.as_ref());
                let done = comp_c.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if let Some(cb) = &opts_c.progress_callback {
                    cb(done, total_pages);
                }
                chunk
            }));
        }
        let mut out = Vec::with_capacity(total_pages);
        for h in render_handles {
            out.push(h.await.map_err(|e| PdfError::TaskJoin(e.to_string()))?);
        }
        out
    } else {
        let mut handles = Vec::with_capacity(total_pages);
        for page_idx in target_pages {
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| PdfError::TaskJoin(e.to_string()))?;
            let doc_c = Arc::clone(&doc);
            let opts_c = Arc::clone(&opts_arc);
            let hdr_c = Arc::clone(&header_resolver);
            let comp_c = Arc::clone(&completed);

            let handle = tokio::task::spawn_blocking(move || {
                let _guard = permit;
                let chunk = process_single_page(&doc_c, page_idx, &opts_c, hdr_c.as_ref())?;
                let done = comp_c.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if let Some(cb) = &opts_c.progress_callback {
                    cb(done, total_pages);
                }
                Ok::<PageChunk, PdfError>(chunk)
            });
            handles.push(handle);
        }

        let mut out = Vec::with_capacity(total_pages);
        for h in handles {
            let res = h.await.map_err(|e| PdfError::TaskJoin(e.to_string()))??;
            out.push(res);
        }
        out
    };

    chunks.sort_by_key(|c| c.metadata.page);

    if opts_arc.page_chunks {
        Ok(MarkdownOutput::Chunks(chunks))
    } else {
        let separator = if opts_arc.page_separators {
            "\n-----\n\n"
        } else {
            "\n"
        };
        let combined = chunks
            .into_iter()
            .map(|c| c.text)
            .collect::<Vec<_>>()
            .join(separator);
        Ok(MarkdownOutput::Text(combined))
    }
}

/// Streaming page-by-page async API yielding `PageChunk` items with bounded concurrency.
pub fn to_markdown_stream(
    doc: Arc<PdfDocument>,
    options: ToMarkdownOptions,
) -> impl Stream<Item = Result<PageChunk>> {
    let target_pages = resolve_target_pages(&doc, &options).unwrap_or_default();
    let opts_arc = Arc::new(options);

    stream::once(async move {
        let doc_c = Arc::clone(&doc);
        let opts_c = Arc::clone(&opts_arc);
        let hdr = tokio::task::spawn_blocking(move || build_header_resolver(&doc_c, &opts_c))
            .await
            .map_err(|e| PdfError::TaskJoin(e.to_string()))??;

        let mut futs = Vec::with_capacity(target_pages.len());
        let sem = Arc::new(Semaphore::new(opts_arc.concurrency.max_page_concurrency));
        for page_idx in target_pages {
            let doc_p = Arc::clone(&doc);
            let opts_p = Arc::clone(&opts_arc);
            let hdr_p = Arc::clone(&hdr);
            let sem_p = Arc::clone(&sem);
            futs.push(async move {
                let _permit = sem_p
                    .acquire_owned()
                    .await
                    .map_err(|e| PdfError::TaskJoin(e.to_string()))?;
                tokio::task::spawn_blocking(move || {
                    process_single_page(&doc_p, page_idx, &opts_p, hdr_p.as_ref())
                })
                .await
                .map_err(|e| PdfError::TaskJoin(e.to_string()))?
            });
        }
        Ok::<_, PdfError>(stream::iter(futs).buffered(opts_arc.concurrency.max_page_concurrency))
    })
    .flat_map(|res| match res {
        Ok(s) => futures::future::Either::Left(s),
        Err(e) => futures::future::Either::Right(stream::once(async move { Err(e) })),
    })
}

/// High-concurrency batch processor executing multiple PDF files in parallel across both
/// document workers (`max_doc_concurrency`) and page workers (`max_page_concurrency`).
pub async fn batch_to_markdown_async(
    paths: Vec<PathBuf>,
    options: ToMarkdownOptions,
) -> Vec<BatchItemResult> {
    let doc_sem = Arc::new(Semaphore::new(options.concurrency.max_doc_concurrency));
    let mut tasks = Vec::with_capacity(paths.len());

    for path in paths {
        let sem = Arc::clone(&doc_sem);
        let opts = options.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.ok();
            let t0 = std::time::Instant::now();
            let output = to_markdown_async(&path, opts).await;
            let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
            BatchItemResult {
                path,
                output,
                elapsed_ms,
            }
        }));
    }

    let mut results = Vec::with_capacity(tasks.len());
    for t in tasks {
        if let Ok(item) = t.await {
            results.push(item);
        }
    }
    results
}
