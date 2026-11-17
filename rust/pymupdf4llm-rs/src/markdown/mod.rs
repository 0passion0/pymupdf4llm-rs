pub mod chunk;
pub mod renderer;

use crate::config::ToMarkdownOptions;
use crate::layout::headers::HeaderResolver;
use crate::layout::LayoutPage;
use crate::pdf::{ParsedPage, PdfDocument};
use chunk::PageChunk;

pub fn render_page_chunk(
    doc: &PdfDocument,
    parsed: &ParsedPage,
    layout: &LayoutPage,
    options: &ToMarkdownOptions,
    header_resolver: &dyn HeaderResolver,
) -> PageChunk {
    renderer::build_page_chunk(doc, parsed, layout, options, header_resolver)
}
