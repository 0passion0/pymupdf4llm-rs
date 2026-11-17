use std::sync::Arc;
use pymupdf4llm_rs::{
    to_markdown, HeaderMode, MarkdownOutput, PdfDocument, TableStrategy, ToMarkdownOptions,
};

/// Constructs a minimal valid binary PDF in-memory for pure-Rust verification.
fn build_synthetic_pdf() -> Vec<u8> {
    let stream_p1 = b"BT\n\
        /F2 20 Tf 72 720 Td (1. Rust PDF Architecture) Tj\n\
        /F1 11 Tf 0 -28 Td (Standard body paragraph with ) Tj\n\
        /F2 11 Tf (bold highlight) Tj\n\
        /F1 11 Tf ( and normal prose.) Tj\n\
        /F3 10 Tf 0 -24 Td (fn main() { println!(\"hello\"); }) Tj\n\
        ET\n\
        0.5 w 72 620 m 300 620 l S\n\
        72 595 m 300 595 l S\n\
        72 570 m 300 570 l S\n\
        72 620 m 72 570 l S\n\
        186 620 m 186 570 l S\n\
        300 620 m 300 570 l S\n\
        BT\n\
        /F2 10 Tf 1 0 0 1 80 603 Tm (Metric) Tj 1 0 0 1 194 603 Tm (Value) Tj\n\
        /F1 10 Tf 1 0 0 1 80 578 Tm (Throughput) Tj 1 0 0 1 194 578 Tm (420 pages/s) Tj\n\
        ET\n";

    assemble_pdf(vec![(stream_p1.to_vec(), None)], false)
}

/// Builds a noisy PDF with Type0 ToUnicode CJK + ligatures, Form XObject, Tr=3 invisible text,
/// faux-bold overprint, rotated diagonal DRAFT watermark, hyphenation, and TWO independent tables.
fn build_noisy_cjk_multitable_pdf() -> Vec<u8> {
    let cmap = b"/CIDInit /ProcSet findresource begin\n\
        12 dict begin begincmap\n\
        /CMapType 2 def\n\
        1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
        2 beginbfchar\n\
        <0001> <4E2D>\n\
        <0002> <6587>\n\
        endbfchar\n\
        1 beginbfrange\n\
        <0010> <0011> [<6D4B> <8BD5>]\n\
        endbfrange\n\
        endcmap CMapName currentdict /CMap defineresource pop end end\n";

    let form_stream = b"BT /F1 11 Tf 1 0 0 1 72 650 Tm (Extracted from Form XObject) Tj ET\n";

    let page_stream = b"BT\n\
        /F2 18 Tf 1 0 0 1 72 720 Tm (Noisy CJK & Multi-Table Report) Tj\n\
        /F4 12 Tf 1 0 0 1 72 695 Tm <0001000200100011> Tj\n\
        /F1 11 Tf 3 Tr 1 0 0 1 72 678 Tm (INVISIBLE_OCR_LAYER) Tj\n\
        /F1 11 Tf 0 Tr 1 0 0 1 72 678 Tm (Clean perform-) Tj\n\
        /F1 11 Tf 1 0 0 1 72 664 Tm (ance metrics with faux-bold:) Tj\n\
        /F2 11 Tf 1 0 0 1 72 630 Tm (FauxBold) Tj\n\
        /F2 11 Tf 1 0 0 1 72.4 630 Tm (FauxBold) Tj\n\
        /F1 36 Tf 0.707 0.707 -0.707 0.707 200 400 Tm (DRAFT) Tj\n\
        ET\n\
        /Form1 Do\n\
        0.5 w 72 560 m 260 560 l S 72 535 m 260 535 l S 72 510 m 260 510 l S\n\
        72 560 m 72 510 l S 166 560 m 166 510 l S 260 560 m 260 510 l S\n\
        BT /F2 10 Tf 1 0 0 1 80 543 Tm (Stage) Tj 1 0 0 1 174 543 Tm (Cost) Tj\n\
        /F1 10 Tf 1 0 0 1 80 518 Tm (Parse) Tj 1 0 0 1 174 518 Tm (1.2ms) Tj ET\n\
        0.5 w 320 560 m 520 560 l S 320 535 m 520 535 l S 320 510 m 520 510 l S\n\
        320 560 m 320 510 l S 420 560 m 420 510 l S 520 560 m 520 510 l S\n\
        BT /F2 10 Tf 1 0 0 1 328 543 Tm (Engine) Tj 1 0 0 1 428 543 Tm (Tables) Tj\n\
        /F1 10 Tf 1 0 0 1 328 518 Tm (UnionFind) Tj 1 0 0 1 428 518 Tm (2) Tj ET\n";

    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.6\n");
    let objs = vec![
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
        "2 0 obj\n<< /Type /Pages /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R /F2 5 0 R /F3 6 0 R /F4 8 0 R >> /XObject << /Form1 10 0 R >> >> /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 7 0 R >>\nendobj\n".to_string(),
        "4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n".to_string(),
        "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>\nendobj\n".to_string(),
        "6 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>\nendobj\n".to_string(),
        format!(
            "7 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            page_stream.len(),
            String::from_utf8_lossy(page_stream)
        ),
        "8 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /NotoSansCJKsc-Regular /Encoding /Identity-H /ToUnicode 9 0 R >>\nendobj\n".to_string(),
        format!(
            "9 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            cmap.len(),
            String::from_utf8_lossy(cmap)
        ),
        format!(
            "10 0 obj\n<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] /Length {} >>\nstream\n{}endstream\nendobj\n",
            form_stream.len(),
            String::from_utf8_lossy(form_stream)
        ),
    ];
    let mut offsets = Vec::new();
    for obj in &objs {
        offsets.push(pdf.len());
        pdf.extend_from_slice(obj.as_bytes());
    }
    let xref_pos = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in offsets {
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objs.len() + 1,
            xref_pos
        )
        .as_bytes(),
    );
    pdf
}

fn assemble_pdf(pages: Vec<(Vec<u8>, Option<i32>)>, _with_inherited: bool) -> Vec<u8> {
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.4\n");
    let mut objs = vec![
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
        "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R /F2 5 0 R /F3 6 0 R >> >> /Contents 7 0 R >>\nendobj\n".to_string(),
        "4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n".to_string(),
        "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>\nendobj\n".to_string(),
        "6 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>\nendobj\n".to_string(),
    ];
    let stream_p1 = &pages[0].0;
    objs.push(format!(
        "7 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
        stream_p1.len(),
        String::from_utf8_lossy(stream_p1)
    ));

    let mut offsets = Vec::new();
    for obj in &objs {
        offsets.push(pdf.len());
        pdf.extend_from_slice(obj.as_bytes());
    }
    let xref_pos = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in offsets {
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objs.len() + 1,
            xref_pos
        )
        .as_bytes(),
    );
    pdf
}

#[test]
fn test_identify_headers_table_and_code_parity() {
    let bytes = build_synthetic_pdf();
    let opts = ToMarkdownOptions::builder()
        .table_strategy(TableStrategy::LinesStrict)
        .page_chunks(true)
        .extract_words(true)
        .hdr_info(HeaderMode::IdentifyHeaders {
            max_levels: 4,
            body_limit: None,
        })
        .build();

    let MarkdownOutput::Chunks(chunks) = to_markdown(bytes.as_slice(), opts).unwrap() else {
        panic!("expected chunks");
    };
    assert_eq!(chunks.len(), 1);
    let chunk = &chunks[0];
    assert_eq!(chunk.metadata.page, 1);
    assert!(chunk.text.contains("# 1. Rust PDF Architecture"));
    assert!(chunk.text.contains("**bold highlight**"));
    assert!(chunk.text.contains("```"));
    assert!(chunk.text.contains("|Metric|Value|"));
    assert_eq!(chunk.tables.len(), 1);
    assert!(!chunk.words.is_empty());
}

#[test]
fn test_cjk_tounicode_form_xobject_hygiene_and_multitable() {
    let bytes = build_noisy_cjk_multitable_pdf();
    let opts = ToMarkdownOptions::builder()
        .table_strategy(TableStrategy::LinesStrict)
        .page_chunks(true)
        .build();

    let MarkdownOutput::Chunks(chunks) = to_markdown(bytes.as_slice(), opts).unwrap() else {
        panic!("expected chunks");
    };
    let chunk = &chunks[0];
    assert!(chunk.text.contains("中文测试"));
    assert!(chunk.text.contains("Extracted from Form XObject"));
    assert!(chunk.text.contains("performance"));
    assert!(!chunk.text.contains("INVISIBLE_OCR_LAYER"));
    assert!(!chunk.text.contains("DRAFT"));
    assert_eq!(chunk.tables.len(), 2);
    assert!(chunk.text.contains("|Stage|Cost|"));
    assert!(chunk.text.contains("|Engine|Tables|"));
    let hygiene = chunk.hygiene.as_ref().expect("hygiene stats");
    assert_eq!(hygiene.invisible_spans_filtered, 1);
    assert_eq!(hygiene.watermarks_filtered, 1);
    assert_eq!(hygiene.faux_bold_deduped, 1);
    assert!(hygiene.dehyphenated_joins >= 1);
}

#[tokio::test]
async fn test_async_concurrent_execution() {
    let bytes = build_synthetic_pdf();
    let doc = Arc::new(PdfDocument::from_bytes(&bytes).unwrap());
    let opts = ToMarkdownOptions::builder()
        .max_page_concurrency(8)
        .page_chunks(false)
        .build();

    let out = pymupdf4llm_rs::async_engine::to_markdown_doc_async(doc, opts)
        .await
        .unwrap();
    let text = out.as_text().unwrap();
    assert!(text.contains("# 1. Rust PDF Architecture"));
    assert!(text.contains("420 pages/s"));

    let global_out = pymupdf4llm_rs::PdfEngine::global()
        .extract_bytes_shared(
            bytes,
            std::sync::Arc::clone(pymupdf4llm_rs::PdfEngine::default_options()),
        )
        .await
        .unwrap();
    assert!(global_out.as_text().unwrap().contains("# 1. Rust PDF Architecture"));
}

#[test]
fn test_closely_spaced_spans_total_order_never_panics() {
    // Regression test for Rust smallsort total-order panic when y-coords chain across an epsilon band
    let mut stream = b"BT\n".to_vec();
    for i in 0..40 {
        let y = 720.0 - (i as f32) * 2.2;
        let x = if i % 2 == 0 { 220.0 } else { 72.0 };
        let line = format!("/F1 10 Tf 1 0 0 1 {x:.1} {y:.1} Tm (Span{i}) Tj\n");
        stream.extend_from_slice(line.as_bytes());
    }
    stream.extend_from_slice(b"ET\n");
    let pdf = assemble_pdf(vec![(stream, None)], false);
    let md = to_markdown(pdf.as_slice(), ToMarkdownOptions::default())
        .unwrap()
        .into_text();
    assert!(md.contains("Span0"));
    assert!(md.contains("Span39"));
}
