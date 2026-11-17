"""Pytest verification suite for pymupdf4llm_rs Python bindings (PyO3 + maturin)."""

from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path

import pytest

try:
    import pymupdf4llm_rs as pm
    from pymupdf4llm_rs import LlamaMarkdownReader, PdfExtractor
except ImportError as exc:
    pytest.skip(f"Native Rust extension not built yet ({exc})", allow_module_level=True)


def build_synthetic_pdf() -> bytes:
    stream = (
        b"BT\n"
        b"/F2 20 Tf 72 720 Td (1. Python Rust Hybrid Architecture) Tj\n"
        b"/F1 11 Tf 0 -28 Td (Standard body paragraph with ) Tj\n"
        b"/F2 11 Tf (reusable engine) Tj\n"
        b"/F1 11 Tf ( and CJK hygiene support.) Tj\n"
        b"/F1 11 Tf 3 Tr 1 0 0 1 72 670 Tm (INVISIBLE_OCR_NOISE) Tj\n"
        b"/F1 36 Tf 0.707 0.707 -0.707 0.707 200 400 Tm (DRAFT) Tj\n"
        b"ET\n"
    )
    objs = [
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        (
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
            "/Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>\nendobj\n"
        ),
        "4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>\nendobj\n",
        (
            f"6 0 obj\n<< /Length {len(stream)} >>\nstream\n"
            f"{stream.decode('latin1')}endstream\nendobj\n"
        ),
    ]
    buf = bytearray(b"%PDF-1.4\n")
    offsets: list[int] = []
    for obj in objs:
        offsets.append(len(buf))
        buf.extend(obj.encode("latin1"))
    xref = len(buf)
    buf.extend(f"xref\n0 {len(objs) + 1}\n0000000000 65535 f \n".encode("ascii"))
    for off in offsets:
        buf.extend(f"{off:010d} 00000 n \n".encode("ascii"))
    buf.extend(
        f"trailer\n<< /Size {len(objs) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode(
            "ascii"
        )
    )
    return bytes(buf)


def test_extractor_reuse_and_with_options(tmp_path: Path) -> None:
    pdf_bytes = build_synthetic_pdf()
    pdf_file = tmp_path / "sample.pdf"
    pdf_file.write_bytes(pdf_bytes)

    extractor = PdfExtractor(
        table_strategy="lines_strict",
        margins=(0.0, 40.0, 0.0, 40.0),
        ignore_invisible_text=True,
        strip_watermarks=True,
        extract_words=True,
        max_pdf_bytes=16 * 1024 * 1024,
    )
    assert extractor.options["table_strategy"] == "lines_strict"
    assert extractor.options["uses_global_engine"] is True

    derived = extractor.with_options(table_strategy="text", ignore_code=True)
    assert derived.options["table_strategy"] == "text"
    assert derived.options["ignore_code"] is True
    assert extractor.options["table_strategy"] == "lines_strict"

    md = extractor.to_markdown(pdf_file)
    assert isinstance(md, str)
    assert "Python Rust Hybrid Architecture" in md
    assert "**reusable engine**" in md
    assert "INVISIBLE_OCR_NOISE" not in md
    assert "DRAFT" not in md

    chunks = extractor.to_chunks_bytes(pdf_bytes)
    assert len(chunks) == 1
    assert chunks[0]["metadata"]["page"] == 1
    assert len(chunks[0]["words"]) > 0

    parsed_json = json.loads(extractor.to_json(pdf_bytes))
    assert parsed_json[0]["metadata"]["page_count"] == 1

    plain_text = extractor.to_text(pdf_bytes)
    assert "**" not in plain_text
    assert "reusable engine" in plain_text


def test_parsed_pdf_parse_once_and_context_manager() -> None:
    pdf_bytes = build_synthetic_pdf()
    extractor = PdfExtractor()

    with extractor.open_bytes(pdf_bytes) as doc:
        assert doc.page_count == 1
        assert doc.metadata()["page_count"] == 1
        md = doc.to_markdown()
        chunks = doc.to_chunks(extract_words=True)
        txt = doc.to_text()

    assert isinstance(md, str)
    assert len(chunks) == 1
    assert "Python Rust Hybrid Architecture" in txt


def test_safety_limits_and_option_validation() -> None:
    pdf_bytes = build_synthetic_pdf()
    tiny_guard = PdfExtractor(max_pdf_bytes=64)
    with pytest.raises(ValueError, match="max_pdf_bytes"):
        tiny_guard.to_markdown_bytes(pdf_bytes)

    with pytest.raises(ValueError, match="Unknown option"):
        PdfExtractor(unknown_option_flag=True)

    for bad_kwargs in (
        {"page_width": 0.0},
        {"page_width": float("nan")},
        {"page_height": -10.0},
        {"page_height": float("inf")},
        {"image_size_limit": -0.1},
        {"image_size_limit": float("nan")},
        {"header_body_limit": 0.0},
    ):
        with pytest.raises(ValueError):
            PdfExtractor(**bad_kwargs)


def test_multithreaded_gil_release_and_batch(tmp_path: Path) -> None:
    pdf_bytes = build_synthetic_pdf()
    files = []
    for i in range(4):
        p = tmp_path / f"doc_{i}.pdf"
        p.write_bytes(pdf_bytes)
        files.append(p)

    extractor = PdfExtractor(max_concurrent_docs=2, max_concurrent_pages=4)
    assert extractor.options["uses_global_engine"] is False

    batch = extractor.batch_to_markdown(files)
    assert len(batch) == 4
    assert all(item["ok"] for item in batch)

    with ThreadPoolExecutor(max_workers=4) as pool:
        results = list(pool.map(lambda _: extractor.to_markdown_bytes(pdf_bytes), range(8)))
    assert all("Python Rust Hybrid Architecture" in r for r in results)

    reader = LlamaMarkdownReader(extractor)
    docs = reader.load_data(files[0], extra_info={"source": "unit-test"})
    assert len(docs) == 1
    assert docs[0]["metadata"]["source"] == "unit-test"
    assert pm.__version__ == "0.4.0"
