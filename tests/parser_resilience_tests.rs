use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;

use pymupdf4llm_rs::{to_markdown, HeaderMode, MarkdownOutput, PdfDocument, ToMarkdownOptions};

fn build_classic_xref_pdf(header: &[u8], objs: &[Vec<u8>], trailer_extra: &str) -> Vec<u8> {
    let mut pdf = Vec::new();
    pdf.extend_from_slice(header);
    let mut offsets = Vec::with_capacity(objs.len());
    for obj in objs {
        offsets.push(pdf.len());
        pdf.extend_from_slice(obj);
    }
    let xref_pos = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in offsets {
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R {} >>\nstartxref\n{}\n%%EOF\n",
            objs.len() + 1,
            trailer_extra,
            xref_pos
        )
        .as_bytes(),
    );
    pdf
}

#[test]
fn test_shifted_header_null_encrypt_wrong_length_and_indirect_filter() {
    let stream1 = b"BT /F1 12 Tf 1 0 0 1 72 720 Tm (Shifted Header Recovered) Tj ET\n";
    let stream2_plain = b"BT /F1 12 Tf 1 0 0 1 72 690 Tm (Indirect Filter OK) Tj ET\n";

    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(stream2_plain).unwrap();
    let stream2_zlib = enc.finish().unwrap();

    let mut obj5 = Vec::new();
    // Deliberately bogus /Length 9999 to exercise endstream verification fallback
    obj5.extend_from_slice(b"5 0 obj\n<< /Length 9999 >>\nstream\n");
    obj5.extend_from_slice(stream1);
    obj5.extend_from_slice(b"endstream\nendobj\n");

    let mut obj8 = Vec::new();
    obj8.extend_from_slice(
        format!(
            "8 0 obj\n<< /Filter 7 0 R /Length {} >>\nstream\n",
            stream2_zlib.len()
        )
        .as_bytes(),
    );
    obj8.extend_from_slice(&stream2_zlib);
    obj8.extend_from_slice(b"\nendstream\nendobj\n");

    let objs: Vec<Vec<u8>> = vec![
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_vec(),
        b"2 0 obj\n<< /Type /Pages /Kids 6 0 R /Count 1 >>\nendobj\n".to_vec(),
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents [5 0 R 8 0 R] >>\nendobj\n".to_vec(),
        b"4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n".to_vec(),
        obj5,
        b"6 0 obj\n[3 0 R]\nendobj\n".to_vec(),
        b"7 0 obj\n/FlateDecode\nendobj\n".to_vec(),
        obj8,
    ];

    let inner_pdf = build_classic_xref_pdf(b"%PDF-1.5\n", &objs, "/Encrypt null");
    let mut shifted_pdf = b"JUNK_DOWNLOAD_PREFIX_123".to_vec();
    shifted_pdf.extend_from_slice(&inner_pdf);

    let md = to_markdown(shifted_pdf.as_slice(), ToMarkdownOptions::default())
        .unwrap()
        .into_text();
    assert!(md.contains("Shifted Header Recovered"), "got: {md}");
    assert!(md.contains("Indirect Filter OK"), "got: {md}");
}

#[test]
fn test_pdf15_xref_stream_and_objstm_unpacking() {
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.5\n");

    let content_stream = b"BT /F1 12 Tf 1 0 0 1 72 720 Tm (Compressed ObjStm Page) Tj ET\n";
    let off5 = pdf.len();
    pdf.extend_from_slice(
        format!(
            "5 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            content_stream.len(),
            String::from_utf8_lossy(content_stream)
        )
        .as_bytes(),
    );

    // Pack objects 1..=4 inside /Type /ObjStm (object 6)
    let o1 = "<< /Type /Catalog /Pages 2 0 R >> ";
    let o2 = "<< /Type /Pages /Kids [3 0 R] /Count 1 >> ";
    let o3 = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >> ";
    let o4 = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";
    let d1 = 0usize;
    let d2 = o1.len();
    let d3 = d2 + o2.len();
    let d4 = d3 + o3.len();
    let header_str = format!("1 {d1} 2 {d2} 3 {d3} 4 {d4} ");
    let first = header_str.len();
    let objstm_payload = format!("{header_str}{o1}{o2}{o3}{o4}");

    let off6 = pdf.len();
    pdf.extend_from_slice(
        format!(
            "6 0 obj\n<< /Type /ObjStm /N 4 /First {} /Length {} >>\nstream\n{}endstream\nendobj\n",
            first,
            objstm_payload.len(),
            objstm_payload
        )
        .as_bytes(),
    );

    // Build /Type /XRef stream (object 7) with /W [1 2 1] for objects 0..=7
    let off7 = pdf.len();
    let mut xref_data: Vec<u8> = Vec::new();
    xref_data.extend_from_slice(&[0, 0, 0, 255]); // 0: free
    xref_data.extend_from_slice(&[2, 0, 6, 0]); // 1: compressed in objstm 6 idx 0
    xref_data.extend_from_slice(&[2, 0, 6, 1]); // 2: compressed in objstm 6 idx 1
    xref_data.extend_from_slice(&[2, 0, 6, 2]); // 3: compressed in objstm 6 idx 2
    xref_data.extend_from_slice(&[2, 0, 6, 3]); // 4: compressed in objstm 6 idx 3
    xref_data.extend_from_slice(&[1, (off5 >> 8) as u8, (off5 & 0xFF) as u8, 0]);
    xref_data.extend_from_slice(&[1, (off6 >> 8) as u8, (off6 & 0xFF) as u8, 0]);
    xref_data.extend_from_slice(&[1, (off7 >> 8) as u8, (off7 & 0xFF) as u8, 0]);

    pdf.extend_from_slice(
        format!(
            "7 0 obj\n<< /Type /XRef /Size 8 /W [1 2 1] /Root 1 0 R /Length {} >>\nstream\n",
            xref_data.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&xref_data);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{off7}\n%%EOF\n").as_bytes());

    let doc = PdfDocument::from_bytes(&pdf).unwrap();
    assert_eq!(doc.page_count(), 1);
    let md = to_markdown(pdf.as_slice(), ToMarkdownOptions::default())
        .unwrap()
        .into_text();
    assert!(md.contains("Compressed ObjStm Page"), "got: {md}");
}

#[test]
fn test_damaged_catalog_page_salvage_and_scientific_notation() {
    let stream = b"BT /F1 1.2e1 Tf 1.0e0 0 0 1e0 7.2e1 7.2e2 Tm (Salvaged Orphan Page) Tj ET\n";
    let pdf = format!(
        "%PDF-1.4\n\
        3 0 obj\n<< /Type /Page /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n\
        4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n\
        5 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n\
        trailer\n<< /Root 999 0 R >>\n%%EOF\n",
        stream.len(),
        String::from_utf8_lossy(stream)
    );

    let doc = PdfDocument::from_bytes(pdf.as_bytes()).unwrap();
    assert_eq!(doc.page_count(), 1);
    let md = to_markdown(pdf.as_bytes(), ToMarkdownOptions::default())
        .unwrap()
        .into_text();
    assert!(md.contains("Salvaged Orphan Page"), "got: {md}");
}

#[test]
fn test_named_outline_destinations_and_full_line_toc_headers() {
    let s1 = b"BT /F1 11 Tf 1 0 0 1 72 720 Tm (Page One Intro) Tj ET\n";
    // Split heading across two spans on the same baseline to test full-line TocHeaders matching
    let s2 = b"BT /F1 11 Tf 1 0 0 1 72 720 Tm (2. ) Tj (Methodology Overview) Tj /F1 11 Tf 1 0 0 1 72 690 Tm (Body details here.) Tj ET\n";

    let objs: Vec<Vec<u8>> = vec![
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Outlines 8 0 R /Names << /Dests 10 0 R >> >>\nendobj\n".to_vec(),
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>\nendobj\n".to_vec(),
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R >>\nendobj\n".to_vec(),
        b"4 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 7 0 R >>\nendobj\n".to_vec(),
        b"5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n".to_vec(),
        format!(
            "6 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            s1.len(),
            String::from_utf8_lossy(s1)
        )
        .into_bytes(),
        format!(
            "7 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
            s2.len(),
            String::from_utf8_lossy(s2)
        )
        .into_bytes(),
        b"8 0 obj\n<< /First 9 0 R /Last 9 0 R >>\nendobj\n".to_vec(),
        b"9 0 obj\n<< /Title (2. Methodology Overview) /Dest (sec_two) >>\nendobj\n".to_vec(),
        b"10 0 obj\n<< /Names [ (sec_two) [4 0 R /Fit] ] >>\nendobj\n".to_vec(),
    ];

    let pdf = build_classic_xref_pdf(b"%PDF-1.6\n", &objs, "");
    let doc = PdfDocument::from_bytes(&pdf).unwrap();
    assert_eq!(doc.get_toc(), &[(1, "2. Methodology Overview".to_string(), 2)]);

    let opts = ToMarkdownOptions::builder()
        .hdr_info(HeaderMode::TocHeaders)
        .page_chunks(true)
        .build();
    let MarkdownOutput::Chunks(chunks) = to_markdown(pdf.as_slice(), opts).unwrap() else {
        panic!("expected chunks");
    };
    assert_eq!(chunks.len(), 2);
    assert!(
        chunks[1].text.contains("# 2. Methodology Overview"),
        "got: {}",
        chunks[1].text
    );
}

#[test]
fn test_utf8_name_escapes_chained_refs_leading_zlib_garbage_and_partial_contents_salvage() {
    // `/#E9#BB#91#E4#BD#93` is UTF-8 for `黑体` (bold Chinese font name)
    let good_stream_plain =
        b"BT /#E9#BB#91#E4#BD#93 11 Tf 1 0 0 1 72 720 Tm (Chained And Salvaged) Tj ET\n";
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(good_stream_plain).unwrap();
    let mut padded_zlib = b"BOM\xEF\xBB\xBF".to_vec();
    padded_zlib.extend_from_slice(&enc.finish().unwrap());

    let mut obj5_corrupt = Vec::new();
    obj5_corrupt
        .extend_from_slice(b"5 0 obj\n<< /Filter /FlateDecode /Length 4 >>\nstream\nDEAD\nendstream\nendobj\n");

    let mut obj6_padded = Vec::new();
    obj6_padded.extend_from_slice(
        format!(
            "6 0 obj\n<< /Filter /FlateDecode /Length {} >>\nstream\n",
            padded_zlib.len()
        )
        .as_bytes(),
    );
    obj6_padded.extend_from_slice(&padded_zlib);
    obj6_padded.extend_from_slice(b"\nendstream\nendobj\n");

    let objs: Vec<Vec<u8>> = vec![
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_vec(),
        // Malformed stray token inside /Kids array should not drop `3 0 R`
        b"2 0 obj\n<< /Type /Pages /Kids [ 3 0 R @@@ ] /Count 1 >>\nendobj\n".to_vec(),
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /#E9#BB#91#E4#BD#93 7 0 R >> >> /Contents [5 0 R 6 0 R] >>\nendobj\n".to_vec(),
        b"4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /#E9#BB#91#E4#BD#93 >>\nendobj\n".to_vec(),
        obj5_corrupt,
        obj6_padded,
        // Chained indirect reference: 7 0 R -> 8 0 R -> 4 0 R
        b"7 0 obj\n8 0 R\nendobj\n".to_vec(),
        b"8 0 obj\n4 0 R\nendobj\n".to_vec(),
    ];

    let pdf = build_classic_xref_pdf(b"%PDF-1.7\n", &objs, "");
    let md = to_markdown(pdf.as_slice(), ToMarkdownOptions::default())
        .unwrap()
        .into_text();
    assert!(
        md.contains("**Chained And Salvaged**"),
        "expected UTF-8 bold font + chained ref + padded zlib + partial Contents salvage, got: {md}"
    );
}
