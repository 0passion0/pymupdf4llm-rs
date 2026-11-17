from __future__ import annotations

from os import PathLike
from typing import Any, Literal, Optional, Sequence, TypedDict, Union, overload

__version__: str

PdfSource = Union[str, PathLike[str], bytes, bytearray, memoryview, "ParsedPdf"]
TableStrategy = Literal["lines_strict", "lines", "text", "explicit"]
HdrInfoMode = Optional[Literal["identify", "toc", "disabled", "none"]]
MarginsType = Union[float, tuple[float, float], tuple[float, float, float, float]]


class PageMetadataDict(TypedDict):
    format: str
    title: str
    author: str
    subject: str
    keywords: str
    creator: str
    producer: str
    creation_date: str
    mod_date: str
    encryption: Optional[str]
    file_path: str
    page_count: int
    page: int


class ExtractedTableDict(TypedDict):
    bbox: list[float]
    rows: int
    columns: int


class ExtractedImageDict(TypedDict, total=False):
    number: int
    bbox: list[float]
    width: int
    height: int
    cs_name: str
    ext: str
    file_name: str


class ExtractedGraphicDict(TypedDict):
    bbox: list[float]


class HygieneStatsDict(TypedDict):
    mojibake_chars_removed: int
    ligatures_expanded: int
    faux_bold_deduped: int
    watermarks_filtered: int
    invisible_spans_filtered: int
    dehyphenated_joins: int
    running_headers_removed: int


class PageChunkDict(TypedDict, total=False):
    metadata: PageMetadataDict
    toc_items: list[tuple[int, str, int]]
    tables: list[ExtractedTableDict]
    images: list[ExtractedImageDict]
    graphics: list[ExtractedGraphicDict]
    text: str
    words: list[tuple[float, float, float, float, str, int, int, int]]
    hygiene: HygieneStatsDict


class BatchItemResultDict(TypedDict):
    path: str
    ok: bool
    output: Union[str, list[PageChunkDict], None]
    error: Optional[str]
    elapsed_ms: float


class ParsedPdf:
    @property
    def page_count(self) -> int: ...
    @property
    def file_path(self) -> Optional[str]: ...
    def metadata(self) -> dict[str, Any]: ...
    def toc(self) -> list[tuple[int, str, int]]: ...
    @overload
    def to_markdown(self, *, page_chunks: Literal[False] = ..., **kwargs: Any) -> str: ...
    @overload
    def to_markdown(self, *, page_chunks: Literal[True], **kwargs: Any) -> list[PageChunkDict]: ...
    def to_markdown(self, **kwargs: Any) -> Union[str, list[PageChunkDict]]: ...
    def to_chunks(self, **kwargs: Any) -> list[PageChunkDict]: ...
    def to_json(self, **kwargs: Any) -> str: ...
    def to_text(self, **kwargs: Any) -> str: ...
    def __enter__(self) -> ParsedPdf: ...
    def __exit__(self, exc_type: Any = ..., exc_val: Any = ..., exc_tb: Any = ...) -> bool: ...


class PdfExtractor:
    def __init__(
        self,
        *,
        pages: Optional[list[int]] = ...,
        hdr_info: HdrInfoMode = ...,
        header_max_levels: int = ...,
        header_body_limit: Optional[float] = ...,
        table_strategy: TableStrategy = ...,
        margins: MarginsType = ...,
        page_chunks: bool = ...,
        page_separators: bool = ...,
        page_width: float = ...,
        page_height: Optional[float] = ...,
        extract_words: bool = ...,
        ignore_code: bool = ...,
        ignore_images: bool = ...,
        ignore_graphics: bool = ...,
        force_text: bool = ...,
        fontsize_limit: float = ...,
        use_glyphs: bool = ...,
        detect_bg_color: bool = ...,
        ignore_alpha: bool = ...,
        header: bool = ...,
        footer: bool = ...,
        dpi: int = ...,
        image_size_limit: float = ...,
        graphics_limit: Optional[int] = ...,
        clean_mojibake: bool = ...,
        dehyphenate: bool = ...,
        dedup_faux_bold: bool = ...,
        ignore_invisible_text: bool = ...,
        strip_watermarks: bool = ...,
        dedup_running_headers: bool = ...,
        max_concurrent_docs: Optional[int] = ...,
        max_concurrent_pages: Optional[int] = ...,
        max_pdf_bytes: int = ...,
        max_pages: Optional[int] = ...,
    ) -> None: ...
    @property
    def options(self) -> dict[str, Any]: ...
    def with_options(self, **kwargs: Any) -> PdfExtractor: ...
    def open(self, source: PdfSource) -> ParsedPdf: ...
    def open_bytes(self, data: Union[bytes, bytearray, memoryview]) -> ParsedPdf: ...
    def to_markdown(self, source: PdfSource, **kwargs: Any) -> Union[str, list[PageChunkDict]]: ...
    def to_markdown_bytes(self, data: Union[bytes, bytearray, memoryview], **kwargs: Any) -> Union[str, list[PageChunkDict]]: ...
    def to_chunks(self, source: PdfSource, **kwargs: Any) -> list[PageChunkDict]: ...
    def to_chunks_bytes(self, data: Union[bytes, bytearray, memoryview], **kwargs: Any) -> list[PageChunkDict]: ...
    def to_json(self, source: PdfSource, **kwargs: Any) -> str: ...
    def to_text(self, source: PdfSource, **kwargs: Any) -> str: ...
    def batch_to_markdown(
        self,
        paths: Sequence[Union[str, PathLike[str]]],
        **kwargs: Any,
    ) -> list[BatchItemResultDict]: ...
    def close(self) -> None: ...
    def __enter__(self) -> PdfExtractor: ...
    def __exit__(self, exc_type: Any = ..., exc_val: Any = ..., exc_tb: Any = ...) -> bool: ...


class LlamaMarkdownReader:
    def __init__(self, extractor: Optional[PdfExtractor] = ..., **options: Any) -> None: ...
    @property
    def extractor(self) -> PdfExtractor: ...
    def load_data(
        self,
        file_path: PdfSource,
        extra_info: Optional[dict[str, Any]] = ...,
        **overrides: Any,
    ) -> list[dict[str, Any]]: ...


def to_markdown(source: PdfSource, **kwargs: Any) -> Union[str, list[PageChunkDict]]: ...
def to_markdown_bytes(data: Union[bytes, bytearray, memoryview], **kwargs: Any) -> Union[str, list[PageChunkDict]]: ...
def to_chunks(source: PdfSource, **kwargs: Any) -> list[PageChunkDict]: ...
def to_json(source: PdfSource, **kwargs: Any) -> str: ...
def to_text(source: PdfSource, **kwargs: Any) -> str: ...
