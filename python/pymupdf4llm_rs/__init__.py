"""
pymupdf4llm-rs Python 封装包。

基于纯 Rust（PyO3 + Tokio + Rayon）实现的轻量、高并发、零 C/C++ 依赖 PDF 转 Markdown 核心库，
提供复用底层引擎资源的 ``PdfExtractor``、单次解析句柄 ``ParsedPdf``、兼容 LlamaIndex 的
``LlamaMarkdownReader`` 以及顶层快捷函数接口。

:author: tongliga
:date: 2026-11-18
"""

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
    """
    兼容 LlamaIndex 风格的 PDF Markdown 读取器，内部复用单个 Rust ``PdfExtractor`` 实例。

    通过在生命周期内持有共享的 Rust 解析引擎与安全限流配置，避免每次加载文档时重复初始化
    线程池与解码器资源。

    :author: tongliga
    :date: 2026-11-18
    """

    def __init__(self, extractor: Optional[PdfExtractor] = None, **options: Any) -> None:
        """
        初始化 ``LlamaMarkdownReader`` 实例。

        :param extractor: 可选的既有 ``PdfExtractor`` 实例；传入后将复用其底层 ``Arc<PdfEngine>``。
        :type extractor: Optional[PdfExtractor]
        :param options: 透传给 ``PdfExtractor`` 或 ``with_options`` 的可选解析与并发配置键值参数。
        :type options: Any
        :raises TypeError: 当 ``options`` 包含不支持的参数名或参数类型不匹配时由底层抛出。
        :raises ValueError: 当传入的安全边界或并发配置参数超出合法范围时由底层抛出。
        :author: tongliga
        :date: 2026-11-18
        """
        # 第一步：根据是否提供既有提取器与覆盖参数，决定直接复用、派生副本或新建实例
        if extractor is not None and options:
            self._extractor = extractor.with_options(**options)
        elif extractor is not None:
            self._extractor = extractor
        else:
            self._extractor = PdfExtractor(**options)

    @property
    def extractor(self) -> PdfExtractor:
        """
        获取当前读取器底层持有的 ``PdfExtractor`` 提取器实例。

        :return: 内部复用的 ``PdfExtractor`` 对象。
        :author: tongliga
        :date: 2026-11-18
        """
        return self._extractor

    def load_data(
        self,
        file_path: Union[str, Path, bytes, bytearray, memoryview, ParsedPdf],
        extra_info: Optional[dict[str, Any]] = None,
        **overrides: Any,
    ) -> list[dict[str, Any]]:
        """
        提取逐页文档记录，返回与 LlamaIndex Document 结构兼容的字典列表。

        :param file_path: 目标 PDF 文件路径、内存字节序列或已打开的 ``ParsedPdf`` 文档句柄。
        :type file_path: Union[str, Path, bytes, bytearray, memoryview, ParsedPdf]
        :param extra_info: 需要合并写入每一页 ``metadata`` 字典中的额外业务元数据，默认为空。
        :type extra_info: Optional[dict[str, Any]]
        :param overrides: 本次调用临时覆盖 ``PdfExtractor`` 默认行为的关键字选项。
        :type overrides: Any
        :return: 按页面顺序组织的文档字典列表，每项包含 ``"text"`` 与 ``"metadata"`` 两个核心字段。
        :raises FileNotFoundError: 当传入的文件路径在磁盘上不存在时由底层抛出。
        :raises ValueError: 当文件体积超出 ``max_pdf_bytes``、页数超出 ``max_pages`` 或 PDF 头部非法时抛出。
        :raises RuntimeError: 当底层 Rust 解析器解压或内容流解码失败时抛出。
        :author: tongliga
        :date: 2026-11-18

        .. code-block:: python

            # return 输出数据结构契约（list[dict[str, Any]]）：
            [
                {
                    "text": "# 标题\\n正文内容...",
                    "metadata": {
                        "page": 1,
                        "page_count": 10,
                        "file_path": "paper.pdf",
                        # 若传入 extra_info，则其键值一并合并至此字典
                    },
                }
            ]
        """
        # 第一步：调用底层 Rust 提取器执行逐页分块解析（执行期间自动释放 GIL）
        chunks = self._extractor.to_chunks(file_path, **overrides)

        # 第二步：构建兼容 LlamaIndex Document 的标准返回列表
        # documents: 汇总所有页面结果的列表，元素结构契约为 {"text": str, "metadata": dict[str, Any]}
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
