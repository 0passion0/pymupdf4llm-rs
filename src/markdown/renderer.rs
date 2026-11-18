use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use unicode_normalization::UnicodeNormalization;

use crate::config::ToMarkdownOptions;
use crate::layout::headers::HeaderResolver;
use crate::layout::{LayoutPage, TextLine};
use crate::markdown::chunk::{
    ExtractedGraphic, ExtractedImage, ExtractedTable, ExtractedWord, PageChunk, PageMetadata,
};
use crate::pdf::{ParsedPage, PdfDocument, TextSpan};
use crate::text_cleaner::{should_dehyphenate_join, HygieneStats};

#[derive(Debug, Clone)]
enum FlowBlock {
    Table {
        y: f32,
        x0: f32,
        x1: f32,
        markdown: String,
    },
    Line {
        y: f32,
        x0: f32,
        x1: f32,
        line: TextLine,
    },
    Image {
        y: f32,
        x0: f32,
        x1: f32,
        markdown: String,
    },
}

pub fn build_page_chunk(
    doc: &PdfDocument,
    parsed: &ParsedPage,
    layout: &LayoutPage,
    options: &ToMarkdownOptions,
    header_resolver: &dyn HeaderResolver,
) -> PageChunk {
    let page_1based = parsed.page_index + 1;
    let file_stem = doc
        .file_path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document.pdf".to_string());

    let mut image_metas = Vec::new();
    let mut image_blocks = Vec::new();

    for (idx, img) in layout.images.iter().enumerate() {
        let ext = options.image_format.extension().to_string();
        let file_name = format!("{}-{}-{}.{}", file_stem, parsed.page_index, idx, ext);

        if options.write_images && !options.embed_images {
            let out_dir = if options.image_path.as_os_str().is_empty() {
                std::path::PathBuf::from(".")
            } else {
                options.image_path.clone()
            };
            let _ = std::fs::create_dir_all(&out_dir);
            let target = out_dir.join(&file_name);
            let _ = std::fs::write(&target, &img.data);
        }

        let md_ref = if options.embed_images {
            let b64 = BASE64.encode(&img.data);
            format!(
                "![](data:{};base64,{})",
                options.image_format.mime_type(),
                b64
            )
        } else if options.write_images {
            let display_path = if options.image_path.as_os_str().is_empty() {
                file_name.clone()
            } else {
                options
                    .image_path
                    .join(&file_name)
                    .to_string_lossy()
                    .into_owned()
            };
            format!("![]({})", display_path)
        } else {
            String::new()
        };

        image_metas.push(ExtractedImage {
            number: idx,
            bbox: [img.bbox.x0, img.bbox.y0, img.bbox.x1, img.bbox.y1],
            width: img.width,
            height: img.height,
            cs_name: img.color_space.clone(),
            ext,
            file_name: if options.write_images {
                Some(file_name)
            } else {
                None
            },
        });

        if !md_ref.is_empty() {
            image_blocks.push(FlowBlock::Image {
                y: img.bbox.y0,
                x0: img.bbox.x0,
                x1: img.bbox.x1,
                markdown: md_ref,
            });
        }
    }

    let mut flow: Vec<FlowBlock> = layout
        .ordered_lines
        .iter()
        .map(|line| FlowBlock::Line {
            y: line.bbox.y0,
            x0: line.bbox.x0,
            x1: line.bbox.x1,
            line: line.clone(),
        })
        .collect();

    let mut media_blocks: Vec<FlowBlock> = layout
        .tables
        .iter()
        .map(|tbl| FlowBlock::Table {
            y: tbl.bbox.y0,
            x0: tbl.bbox.x0,
            x1: tbl.bbox.x1,
            markdown: tbl.markdown.clone(),
        })
        .chain(image_blocks)
        .collect();

    media_blocks.sort_by(|a, b| {
        let ya = match a {
            FlowBlock::Table { y, .. } | FlowBlock::Line { y, .. } | FlowBlock::Image { y, .. } => {
                *y
            }
        };
        let yb = match b {
            FlowBlock::Table { y, .. } | FlowBlock::Line { y, .. } | FlowBlock::Image { y, .. } => {
                *y
            }
        };
        ya.total_cmp(&yb)
    });

    for mb in media_blocks {
        let (my, mx0, mx1) = match &mb {
            FlowBlock::Table { y, x0, x1, .. }
            | FlowBlock::Line { y, x0, x1, .. }
            | FlowBlock::Image { y, x0, x1, .. } => (*y, *x0, *x1),
        };
        let insert_idx = flow
            .iter()
            .position(|f| {
                let (fy, fx0, fx1) = match f {
                    FlowBlock::Table { y, x0, x1, .. }
                    | FlowBlock::Line { y, x0, x1, .. }
                    | FlowBlock::Image { y, x0, x1, .. } => (*y, *x0, *x1),
                };
                fx0 < mx1 && fx1 > mx0 && fy > my
            })
            .unwrap_or(flow.len());
        flow.insert(insert_idx, mb);
    }

    let mut hygiene = layout.hygiene_stats.clone();
    let md_text = render_flow_blocks(
        &flow,
        parsed.page_index,
        options,
        header_resolver,
        &mut hygiene,
    );

    let toc_items = doc
        .toc
        .iter()
        .filter(|(_, _, p)| *p == page_1based)
        .cloned()
        .collect();

    let tables = layout
        .tables
        .iter()
        .map(|t| ExtractedTable {
            bbox: [t.bbox.x0, t.bbox.y0, t.bbox.x1, t.bbox.y1],
            rows: t.rows,
            columns: t.cols,
        })
        .collect();

    let graphics = layout
        .graphics
        .iter()
        .map(|g| ExtractedGraphic {
            bbox: [g.x0, g.y0, g.x1, g.y1],
        })
        .collect();

    let words = if options.extract_words {
        extract_word_boxes(&layout.ordered_lines)
    } else {
        Vec::new()
    };

    PageChunk {
        metadata: PageMetadata {
            format: doc.metadata.format.clone(),
            title: doc.metadata.title.clone(),
            author: doc.metadata.author.clone(),
            subject: doc.metadata.subject.clone(),
            keywords: doc.metadata.keywords.clone(),
            creator: doc.metadata.creator.clone(),
            producer: doc.metadata.producer.clone(),
            creation_date: doc.metadata.creation_date.clone(),
            mod_date: doc.metadata.mod_date.clone(),
            encryption: doc.metadata.encryption.clone(),
            file_path: doc
                .file_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            page_count: doc.page_count(),
            page: page_1based,
        },
        toc_items,
        tables,
        images: image_metas,
        graphics,
        text: md_text,
        words,
        hygiene: Some(hygiene),
    }
}

fn render_flow_blocks(
    blocks: &[FlowBlock],
    page_idx: usize,
    options: &ToMarkdownOptions,
    header_resolver: &dyn HeaderResolver,
    hygiene: &mut HygieneStats,
) -> String {
    let mut out = String::new();
    let mut in_code_block = false;
    let mut prev_y_end: Option<f32> = None;
    let mut prev_line_text: Option<String> = None;

    for block in blocks {
        match block {
            FlowBlock::Table { markdown, .. } => {
                if in_code_block {
                    out.push_str("```\n\n");
                    in_code_block = false;
                }
                if !out.is_empty() && !out.ends_with("\n\n") {
                    out.push('\n');
                }
                out.push_str(markdown);
                out.push_str("\n\n");
                prev_y_end = None;
                prev_line_text = None;
            }
            FlowBlock::Image { markdown, .. } => {
                if in_code_block {
                    out.push_str("```\n\n");
                    in_code_block = false;
                }
                if !out.is_empty() && !out.ends_with("\n\n") {
                    out.push('\n');
                }
                out.push_str(markdown);
                out.push_str("\n\n");
                prev_y_end = None;
                prev_line_text = None;
            }
            FlowBlock::Line { line, .. } => {
                let trimmed = line.text.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let header_prefix = line
                    .spans
                    .first()
                    .map(|s| header_resolver.get_header_id(s, page_idx))
                    .unwrap_or_default();

                if !options.ignore_code && line.is_all_monospace && header_prefix.is_empty() {
                    if !in_code_block {
                        if !out.is_empty() && !out.ends_with("\n\n") {
                            out.push('\n');
                        }
                        out.push_str("```\n");
                        in_code_block = true;
                    }
                    out.push_str(trimmed);
                    out.push('\n');
                    prev_y_end = Some(line.bbox.y1);
                    prev_line_text = None;
                    continue;
                } else if in_code_block {
                    out.push_str("```\n\n");
                    in_code_block = false;
                }

                if !header_prefix.is_empty() {
                    if !out.is_empty() && !out.ends_with("\n\n") {
                        out.push('\n');
                    }
                    out.push_str(&header_prefix);
                    out.push_str(trimmed);
                    out.push_str("\n\n");
                    prev_y_end = Some(line.bbox.y1);
                    prev_line_text = None;
                    continue;
                }

                if let Some(rest) = strip_bullet_prefix(trimmed) {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("- ");
                    out.push_str(&format_styled_spans(&line.spans, Some(rest), options));
                    out.push('\n');
                    prev_y_end = Some(line.bbox.y1);
                    prev_line_text = None;
                    continue;
                }

                let mut joined_hyphen = false;
                if let Some(py) = prev_y_end {
                    let gap = line.bbox.y0 - py;
                    if gap > line.max_font_size * 0.85 {
                        if !out.ends_with("\n\n") {
                            if !out.ends_with('\n') {
                                out.push('\n');
                            }
                            out.push('\n');
                        }
                    } else if options.hygiene.dehyphenate
                        && prev_line_text
                            .as_deref()
                            .is_some_and(|pl| should_dehyphenate_join(pl, trimmed))
                        && out.ends_with('-')
                    {
                        out.pop();
                        joined_hyphen = true;
                        hygiene.dehyphenated_joins += 1;
                    } else if !out.is_empty() && !out.ends_with('\n') && !out.ends_with(' ') {
                        out.push(' ');
                    }
                }
                let _ = joined_hyphen;

                out.push_str(&format_styled_spans(&line.spans, None, options));
                prev_y_end = Some(line.bbox.y1);
                prev_line_text = Some(trimmed.to_string());
            }
        }
    }

    if in_code_block {
        out.push_str("```\n");
    }

    while out.ends_with('\n') {
        out.pop();
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn format_single_styled_span(
    s: &TextSpan,
    options: &ToMarkdownOptions,
) -> (String, String, String) {
    let raw = &s.text;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return (String::new(), raw.clone(), String::new());
    }
    let lead_ws = raw[..raw.len() - raw.trim_start().len()].to_string();
    let trail_ws = raw[raw.trim_end().len()..].to_string();

    let styled = if !options.ignore_code && s.is_monospace {
        format!("`{trimmed}`")
    } else if s.is_bold && s.is_italic {
        format!("***{trimmed}***")
    } else if s.is_bold {
        format!("**{trimmed}**")
    } else if s.is_italic {
        format!("*{trimmed}*")
    } else if s.is_strikeout {
        format!("~~{trimmed}~~")
    } else {
        trimmed.to_string()
    };

    (lead_ws, styled, trail_ws)
}

fn format_styled_spans(
    spans: &[TextSpan],
    override_plain: Option<&str>,
    options: &ToMarkdownOptions,
) -> String {
    if let Some(plain) = override_plain {
        return plain.to_string();
    }
    let mut out = String::new();
    let mut i = 0usize;

    while i < spans.len() {
        let s = &spans[i];
        if i > 0 {
            let gap = s.bbox.x0 - spans[i - 1].bbox.x1;
            if gap > s.font_size * 0.12 && !out.ends_with(' ') && !s.text.starts_with(' ') {
                out.push(' ');
            }
        }

        // Merge consecutive spans sharing the same hyperlink URI into a single `[text](uri)` block
        if let Some(uri) = &s.link_uri {
            let mut j = i + 1;
            while j < spans.len() && spans[j].link_uri.as_deref() == Some(uri.as_str()) {
                j += 1;
            }
            let mut combined_inner = String::new();
            let mut lead_ws = String::new();
            let mut trail_ws = String::new();
            for k in i..j {
                let sk = &spans[k];
                if k > i {
                    let gap = sk.bbox.x0 - spans[k - 1].bbox.x1;
                    if gap > sk.font_size * 0.12
                        && !combined_inner.ends_with(' ')
                        && !sk.text.starts_with(' ')
                    {
                        combined_inner.push(' ');
                    }
                }
                let (lw, styled, tw) = format_single_styled_span(sk, options);
                if k == i {
                    lead_ws = lw;
                } else {
                    combined_inner.push_str(&lw);
                }
                combined_inner.push_str(&styled);
                if k + 1 == j {
                    trail_ws = tw;
                } else {
                    combined_inner.push_str(&tw);
                }
            }
            out.push_str(&lead_ws);
            if combined_inner.trim().is_empty() {
                out.push_str(&combined_inner);
            } else {
                out.push_str(&format!("[{}]({uri})", combined_inner.trim()));
            }
            out.push_str(&trail_ws);
            i = j;
            continue;
        }

        let (lead_ws, styled, trail_ws) = format_single_styled_span(s, options);
        out.push_str(&lead_ws);
        out.push_str(&styled);
        out.push_str(&trail_ws);
        i += 1;
    }
    if !out.is_ascii() {
        out = out.nfc().collect();
    }
    out
}

fn strip_bullet_prefix(s: &str) -> Option<&str> {
    for prefix in ["• ", "◦ ", "▪ ", "‣ ", "- ", "* "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return Some(rest.trim_start());
        }
    }
    None
}

fn extract_word_boxes(lines: &[TextLine]) -> Vec<ExtractedWord> {
    let mut words = Vec::new();
    for (line_idx, line) in lines.iter().enumerate() {
        let mut word_idx = 0usize;
        for span in &line.spans {
            let total_chars = span.text.chars().count().max(1) as f32;
            let span_w = span.bbox.width();
            let mut offset = 0usize;
            for part in span.text.split_whitespace() {
                if let Some(rel) = span.text[offset..].find(part) {
                    let char_start = span.text[..offset + rel].chars().count() as f32;
                    let char_end = char_start + part.chars().count() as f32;
                    let wx0 = span.bbox.x0 + (char_start / total_chars) * span_w;
                    let wx1 = span.bbox.x0 + (char_end / total_chars) * span_w;
                    words.push((
                        wx0,
                        span.bbox.y0,
                        wx1,
                        span.bbox.y1,
                        part.to_string(),
                        0,
                        line_idx,
                        word_idx,
                    ));
                    word_idx += 1;
                    offset += rel + part.len();
                }
            }
        }
    }
    words
}

pub fn strip_markdown_syntax(md: &str) -> String {
    md.lines()
        .map(|l| {
            let l = l.trim_start_matches('#').trim_start();
            l.replace("**", "")
                .replace("***", "")
                .replace("~~", "")
                .replace('`', "")
        })
        .collect::<Vec<_>>()
        .join("\n")
}
