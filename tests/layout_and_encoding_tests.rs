use pymupdf4llm_rs::pdf::encoding::{glyph_name_to_unicode, parse_tounicode_cmap};
use pymupdf4llm_rs::{to_markdown, MarkdownOutput, PdfDocument, TableStrategy, ToMarkdownOptions};

fn build_pdf(objs: &[String]) -> Vec<u8> {
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.6\n");
    let mut offsets = Vec::with_capacity(objs.len());
    for obj in objs {
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
fn test_nonzero_cropbox_actual_text_and_tj_coalescing() {
    let stream = b"BT\n\
        /F1 12 Tf 1 0 0 1 150 650 Tm [(H) -10 (e) 15 (l) -5 (lo) -250 (Wo) -10 (rld)] TJ\n\
        /Span << /ActualText (ActualText Override) >> BDC\n\
        /F1 12 Tf 1 0 0 1 150 620 Tm (GARBAGE_GLYPHS) Tj\n\
        EMC\n\
        /Span /MC1 BDC\n\
        /F1 12 Tf 1 0 0 1 150 590 Tm (RAW_PROP_GLYPHS) Tj\n\
        EMC\n\
        ET\n";

    let objs = vec![
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
        "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /CropBox [100 100 500 700] /Resources << /Font << /F1 4 0 R >> /Properties << /MC1 << /ActualText (Prop ActualText) >> >> >> /Contents 5 0 R >>\nendobj\n".to_string(),
        "4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n".to_string(),
        format!(
            "5 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            stream.len(),
            String::from_utf8_lossy(stream)
        ),
    ];

    let pdf = build_pdf(&objs);
    let doc = PdfDocument::from_bytes(&pdf).unwrap();
    let parsed = doc.parse_page(0, &ToMarkdownOptions::default()).unwrap();
    assert!((parsed.crop_box.x0 - 0.0).abs() < 1e-3);
    assert!((parsed.crop_box.y0 - 0.0).abs() < 1e-3);
    assert!((parsed.crop_box.width() - 400.0).abs() < 1e-3);
    assert!((parsed.crop_box.height() - 600.0).abs() < 1e-3);

    let md = to_markdown(pdf.as_slice(), ToMarkdownOptions::default())
        .unwrap()
        .into_text();
    assert!(md.contains("Hello World"), "got: {md}");
    assert!(md.contains("ActualText Override"), "got: {md}");
    assert!(!md.contains("GARBAGE_GLYPHS"), "got: {md}");
    assert!(md.contains("Prop ActualText"), "got: {md}");
    assert!(!md.contains("RAW_PROP_GLYPHS"), "got: {md}");
}

#[test]
fn test_bfrange_ligatures_tex_ot1_cyrillic_and_type3_fontmatrix() {
    // Multi-code-unit sequential bfrange (e.g. ligature `<00660069>` -> "fi", `<0066006A>` -> "fj")
    let cmap = parse_tounicode_cmap(
        b"1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
          1 beginbfrange <0001> <0002> <00660069> endbfrange\n",
    );
    assert_eq!(cmap.mappings.get(&1).map(String::as_str), Some("fi"));
    assert_eq!(cmap.mappings.get(&2).map(String::as_str), Some("fj"));

    // Cyrillic AGL `afii` glyph names
    assert_eq!(
        glyph_name_to_unicode("afii10017", "Times").as_deref(),
        Some("А")
    );
    assert_eq!(
        glyph_name_to_unicode("afii10097", "Times").as_deref(),
        Some("я")
    );

    // End-to-end TeX OT1 (`CMR10`) + Type3 `FontMatrix` width scaling
    let stream = b"BT\n\
        /F1 11 Tf 1 0 0 1 72 720 Tm (\\000 \\014nd \\015ow) Tj\n\
        /F2 11 Tf 1 0 0 1 72 695 Tm (AB) Tj\n\
        ET\n";

    let objs = vec![
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
        "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>\nendobj\n".to_string(),
        "4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /CMR10 >>\nendobj\n".to_string(),
        "5 0 obj\n<< /Type /Font /Subtype /Type3 /FontMatrix [0.001 0 0 0.001 0 0] /FirstChar 65 /LastChar 66 /Widths [600 600] >>\nendobj\n".to_string(),
        format!(
            "6 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            stream.len(),
            String::from_utf8_lossy(stream)
        ),
    ];

    let pdf = build_pdf(&objs);
    let md = to_markdown(pdf.as_slice(), ToMarkdownOptions::default())
        .unwrap()
        .into_text();
    assert!(md.contains("Γ find flow"), "got: {md}");
    assert!(md.contains("AB"), "got: {md}");
}

#[test]
fn test_xycut_spanning_band_two_column_layout() {
    let stream = b"BT\n\
        /F2 16 Tf 1 0 0 1 72 740 Tm (Full Width Paper Title Spanning Across Both Columns) Tj\n\
        /F1 11 Tf 1 0 0 1 330 680 Tm (Right Col Line 1) Tj\n\
        /F1 11 Tf 1 0 0 1 72 680 Tm (Left Col Line 1) Tj\n\
        /F1 11 Tf 1 0 0 1 330 660 Tm (Right Col Line 2) Tj\n\
        /F1 11 Tf 1 0 0 1 72 660 Tm (Left Col Line 2) Tj\n\
        /F1 11 Tf 1 0 0 1 72 580 Tm (Spanning Bottom Summary Across Full Page Width Covering Both Columns) Tj\n\
        ET\n";

    let objs = vec![
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
        "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>\nendobj\n".to_string(),
        "4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n".to_string(),
        "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>\nendobj\n".to_string(),
        format!(
            "6 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            stream.len(),
            String::from_utf8_lossy(stream)
        ),
    ];

    let pdf = build_pdf(&objs);
    let md = to_markdown(pdf.as_slice(), ToMarkdownOptions::default())
        .unwrap()
        .into_text();

    let pos_title = md.find("Full Width Paper Title").expect("title");
    let pos_l1 = md.find("Left Col Line 1").expect("left 1");
    let pos_l2 = md.find("Left Col Line 2").expect("left 2");
    let pos_r1 = md.find("Right Col Line 1").expect("right 1");
    let pos_r2 = md.find("Right Col Line 2").expect("right 2");
    let pos_bottom = md.find("Spanning Bottom Summary").expect("bottom");

    assert!(pos_title < pos_l1, "title before left col: {md}");
    assert!(pos_l1 < pos_l2, "left1 before left2: {md}");
    assert!(pos_l2 < pos_r1, "left col before right col: {md}");
    assert!(pos_r1 < pos_r2, "right1 before right2: {md}");
    assert!(pos_r2 < pos_bottom, "right col before bottom band: {md}");
}

#[test]
fn test_three_line_academic_booktabs_table() {
    let stream = b"0.8 w\n\
        72 620 m 450 620 l S\n\
        72 595 m 450 595 l S\n\
        72 545 m 450 545 l S\n\
        BT\n\
        /F2 10 Tf 1 0 0 1 80 603 Tm (Model) Tj 1 0 0 1 210 603 Tm (Accuracy) Tj 1 0 0 1 340 603 Tm (Latency) Tj\n\
        /F1 10 Tf 1 0 0 1 80 578 Tm (Alpha) Tj 1 0 0 1 210 578 Tm (94.2%) Tj 1 0 0 1 340 578 Tm (12ms) Tj\n\
        /F1 10 Tf 1 0 0 1 80 555 Tm (Beta) Tj 1 0 0 1 210 555 Tm (96.8%) Tj 1 0 0 1 340 555 Tm (15ms) Tj\n\
        ET\n";

    let objs = vec![
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
        "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>\nendobj\n".to_string(),
        "4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n".to_string(),
        "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>\nendobj\n".to_string(),
        format!(
            "6 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            stream.len(),
            String::from_utf8_lossy(stream)
        ),
    ];

    let pdf = build_pdf(&objs);
    let opts = ToMarkdownOptions::builder()
        .table_strategy(TableStrategy::LinesStrict)
        .page_chunks(true)
        .build();
    let MarkdownOutput::Chunks(chunks) = to_markdown(pdf.as_slice(), opts).unwrap() else {
        panic!("expected chunks");
    };
    let chunk = &chunks[0];
    assert_eq!(chunk.tables.len(), 1, "text: {}", chunk.text);
    assert!(
        chunk.text.contains("|Model|Accuracy|Latency|"),
        "got: {}",
        chunk.text
    );
    assert!(
        chunk.text.contains("|Alpha|94.2%|12ms|"),
        "got: {}",
        chunk.text
    );
    assert!(
        chunk.text.contains("|Beta|96.8%|15ms|"),
        "got: {}",
        chunk.text
    );
}

#[test]
fn test_cjk_line_join_chinese_watermark_faux_bold_and_merged_style_runs() {
    let cmap = b"/CIDInit /ProcSet findresource begin\n\
        12 dict begin begincmap\n\
        1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
        10 beginbfchar\n\
        <0001> <4E2D>\n\
        <0002> <6587>\n\
        <0003> <6D4B>\n\
        <0004> <8BD5>\n\
        <0005> <6BB5>\n\
        <0006> <843D>\n\
        <0007> <62FC>\n\
        <0008> <63A5>\n\
        <0009> <673A>\n\
        <000A> <5BC6>\n\
        endbfchar\n\
        endcmap end end\n";

    let mut stream = String::from(
        "BT\n\
        /F3 11 Tf 1 0 0 1 72 720 Tm <0001000200030004> Tj\n\
        /F3 11 Tf 1 0 0 1 72 706 Tm <0005000600070008> Tj\n\
        /F3 36 Tf 0.707 0.707 -0.707 0.707 200 400 Tm <0009000A> Tj\n\
        /F2 11 Tf 1 0 0 1 72 670 Tm (Merged) Tj 1 0 0 1 116 670 Tm (Bold) Tj\n\
        /F2 11 Tf 1 0 0 1 72 640 Tm (OverprintTarget) Tj\n",
    );
    // Emit 20 intervening spans to prove spatial-bucket faux-bold dedup works across the whole page
    for i in 0..20 {
        let y = 610 - i * 16;
        stream.push_str(&format!("/F1 11 Tf 1 0 0 1 72 {y} Tm (Line{i}) Tj\n"));
    }
    // Second-pass faux-bold overprint of OverprintTarget at +0.4pt offset
    stream.push_str("/F2 11 Tf 1 0 0 1 72.4 640 Tm (OverprintTarget) Tj\nET\n");

    let objs = vec![
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
        "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R /F2 5 0 R /F3 6 0 R >> >> /Contents 8 0 R >>\nendobj\n".to_string(),
        "4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n".to_string(),
        "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>\nendobj\n".to_string(),
        "6 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /NotoSansCJKsc-Regular /Encoding /Identity-H /ToUnicode 7 0 R >>\nendobj\n".to_string(),
        format!(
            "7 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            cmap.len(),
            String::from_utf8_lossy(cmap)
        ),
        format!(
            "8 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            stream.len(),
            stream
        ),
    ];

    let pdf = build_pdf(&objs);
    let MarkdownOutput::Chunks(chunks) = to_markdown(
        pdf.as_slice(),
        ToMarkdownOptions::builder().page_chunks(true).build(),
    )
    .unwrap() else {
        panic!("expected chunks");
    };
    let chunk = &chunks[0];
    assert!(
        chunk.text.contains("中文测试段落拼接"),
        "got: {}",
        chunk.text
    );
    assert!(!chunk.text.contains("机密"), "got: {}", chunk.text);
    assert!(
        chunk.text.contains("**Merged Bold**"),
        "got: {}",
        chunk.text
    );
    let hygiene = chunk.hygiene.as_ref().expect("hygiene");
    assert_eq!(hygiene.watermarks_filtered, 1);
    assert_eq!(hygiene.faux_bold_deduped, 1);
}

#[test]
fn test_descendant_font_descriptor_utf8_cmap_and_variable_length_cmap_lookup() {
    // CMap containing `/CIDSystemInfo << ... >>` dict + both 1-byte and 2-byte codespaces
    let cmap = parse_tounicode_cmap(
        b"/CIDInit /ProcSet findresource begin\n\
          /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
          2 begincodespacerange\n\
          <00> <7F>\n\
          <8140> <FEFE>\n\
          endcodespacerange\n\
          2 beginbfchar\n\
          <01> <0041>\n\
          <0001> <4E2D>\n\
          endbfchar\n\
          endcmap end end\n",
    );
    assert_eq!(cmap.lookup(1, 0x01), Some("A"));
    assert_eq!(cmap.lookup(2, 0x0001), Some("中"));

    // Type0 font with `/Encoding /UniGB-UTF8-H` and `/FontDescriptor` on DescendantFont
    let utf8_hex = "E6B58BE8AF95E7B297E6969C"; // "测试粗斜" in UTF-8
    let stream = format!("BT /F1 11 Tf 1 0 0 1 72 720 Tm <{utf8_hex}> Tj ET\n");

    let objs = vec![
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
        "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 7 0 R >>\nendobj\n".to_string(),
        "4 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /CustomCJK /Encoding /UniGB-UTF8-H /DescendantFonts [5 0 R] >>\nendobj\n".to_string(),
        "5 0 obj\n<< /Type /Font /Subtype /CIDFontType2 /BaseFont /CustomCJK /FontDescriptor 6 0 R /DW 1000 >>\nendobj\n".to_string(),
        "6 0 obj\n<< /Type /FontDescriptor /FontWeight 700 /ItalicAngle -12 >>\nendobj\n".to_string(),
        format!(
            "7 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            stream.len(),
            stream
        ),
    ];

    let pdf = build_pdf(&objs);
    let md = to_markdown(pdf.as_slice(), ToMarkdownOptions::default())
        .unwrap()
        .into_text();
    assert!(
        md.contains("***测试粗斜***"),
        "expected DescendantFont bold+italic and UniGB-UTF8-H decoding, got: {md}"
    );
}

#[test]
fn test_thin_filled_rect_table_strikethrough_tr2_bold_hidden_bg_and_styled_bullets() {
    // Word/Chrome style thin filled-rect table rules (0.8pt height) + strikethrough + 2 Tr bold
    // + white-on-white invisible text + styled bullet + ordered list + first-line indent
    let stream = b"0 0 0 rg\n\
        72 700 320 0.8 re f\n\
        72 675 320 0.8 re f\n\
        72 625 320 0.8 re f\n\
        BT\n\
        /F2 10 Tf 1 0 0 1 80 683 Tm (Metric) Tj 1 0 0 1 240 683 Tm (Score) Tj\n\
        /F1 10 Tf 1 0 0 1 80 658 Tm (Precision) Tj 1 0 0 1 240 658 Tm (98.5%) Tj\n\
        /F1 10 Tf 1 0 0 1 80 636 Tm (Recall) Tj 1 0 0 1 240 636 Tm (97.2%) Tj\n\
        ET\n\
        0.6 w 72 584 m 150 584 l S\n\
        BT\n\
        /F1 11 Tf 1 0 0 1 72 580 Tm (Deprecated) Tj\n\
        2 Tr 0.5 w /F1 11 Tf 1 0 0 1 72 555 Tm (FillStrokeBold) Tj\n\
        0 Tr\n\
        ET\n\
        1 1 1 rg 72 515 120 20 re f\n\
        BT\n\
        1 1 1 rg /F1 11 Tf 1 0 0 1 76 522 Tm (HiddenWhiteOnWhite) Tj\n\
        0 0 0 rg\n\
        /F1 11 Tf 1 0 0 1 72 490 Tm (\\225) Tj /F2 11 Tf 1 0 0 1 86 490 Tm (Critical Item) Tj\n\
        /F1 11 Tf 1 0 0 1 72 470 Tm (1. First step.) Tj\n\
        /F1 11 Tf 1 0 0 1 72 456 Tm (2. Second step.) Tj\n\
        /F1 11 Tf 1 0 0 1 72 430 Tm (First paragraph ends here.) Tj\n\
        /F1 11 Tf 1 0 0 1 96 416 Tm (Indented second paragraph.) Tj\n\
        ET\n";

    let objs = vec![
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
        "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>\nendobj\n".to_string(),
        "4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>\nendobj\n".to_string(),
        "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>\nendobj\n".to_string(),
        format!(
            "6 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            stream.len(),
            String::from_utf8_lossy(stream)
        ),
    ];

    let pdf = build_pdf(&objs);
    let MarkdownOutput::Chunks(chunks) = to_markdown(
        pdf.as_slice(),
        ToMarkdownOptions::builder()
            .table_strategy(TableStrategy::LinesStrict)
            .page_chunks(true)
            .build(),
    )
    .unwrap() else {
        panic!("expected chunks");
    };
    let md = &chunks[0].text;
    assert_eq!(chunks[0].tables.len(), 1, "got: {md}");
    assert!(md.contains("|Metric|Score|"), "got: {md}");
    assert!(md.contains("~~Deprecated~~"), "got: {md}");
    assert!(md.contains("**FillStrokeBold**"), "got: {md}");
    assert!(!md.contains("HiddenWhiteOnWhite"), "got: {md}");
    assert!(md.contains("- **Critical Item**"), "got: {md}");
    assert!(md.contains("1. First step.\n2. Second step."), "got: {md}");
    assert!(
        md.contains("First paragraph ends here.\n\nIndented second paragraph."),
        "got: {md}"
    );
}
