"""pymupdf4llm-rs: Lightweight, safe, high-concurrency PDF-to-Markdown powered by pure Rust."""

from __future__ import annotations

from pathlib import Path
from typing import Any, Optional, Union

from ._native import (
    ParsedPdf,
    PdfExtractor,
    __version__,
    to_chunks,
    to_json,
    to_markdown,
    to_markdown_bytes,
    to_text,
)


class LlamaMarkdownReader:
    """Drop-in LlamaIndex-style document reader reusing a single Rust `PdfExtractor`."""

    def __init__(self, extractor: Optional[PdfExtractor] = None, **options: Any) -> None:
        if extractor is not None and options:
            self._extractor = extractor.with_options(**options)
        elif extractor is not None:
            self._extractor = extractor
        else:
            self._extractor = PdfExtractor(**options)

    @property
    def extractor(self) -> PdfExtractor:
        return self._extractor

    def load_data(
        self,
        file_path: Union[str, Path, bytes, bytearray, memoryview, ParsedPdf],
        extra_info: Optional[dict[str, Any]] = None,
        **overrides: Any,
    ) -> list[dict[str, Any]]:
        """Extracts per-page document records compatible with LlamaIndex Document schema."""
        chunks = self._extractor.to_chunks(file_path, **overrides)
        documents: list[dict[str, Any]] = []
        for chunk in chunks:
            meta = dict(chunk["metadata"])
            if extra_info:
                meta.update(extra_info)
            documents.append(
                {
                    "text": chunk["text"],
                    "metadata": meta,
                }
            )
        return documents


__all__ = [
    "__version__",
    "PdfExtractor",
    "ParsedPdf",
    "LlamaMarkdownReader",
    "to_markdown",
    "to_markdown_bytes",
    "to_chunks",
    "to_json",
    "to_text",
]
