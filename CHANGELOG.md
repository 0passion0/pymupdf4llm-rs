# Changelog / 更新日志

All notable changes to `pymupdf4llm-rs` are documented in this file.  
本文档记录 `pymupdf4llm-rs` 纯 Rust 核心库的重要版本变更。

---

## [0.4.0] - 2026-11-16

### English
- **PDF Format & Encoding Compatibility**:
  - Added `/Pages` tree inheritance for `/MediaBox`, `/CropBox`, `/Rotate`, and `/Resources`.
  - Added `/Subtype /Form` XObject recursive stream execution with `/Matrix` & font stack isolation.
  - Added pure-Rust `LZWDecode` decompression (9–12 bit variable codes with `EarlyChange`).
  - Added `Type0`/CID `/W` width parsing, array-form `beginbfrange` `ToUnicode` CMap decoding, `MacRomanEncoding`, and 70+ Adobe Glyph names.
  - Added `/Rotate` (0°/90°/180°/270°) page coordinate normalization and `DashMap` lock-free font caching.
- **Text Hygiene & Noise Cleanup**:
  - Added `TextHygieneConfig` & `HygieneStats` diagnostics (`clean_mojibake`, `dehyphenate`, `dedup_faux_bold`, `ignore_invisible_text`, `strip_watermarks`, `dedup_running_headers`).
  - Stripped `3 Tr` invisible OCR overlays, diagonal/low-alpha watermarks, unmapped PUA/control mojibake, and faux-bold overprint duplicates.
  - Added ligature expansion (`ﬀ`, `ﬁ`, `ﬂ`, `ﬃ`, `ﬄ`, `ﬅ`), PUA bullet normalization, line-wrap de-hyphenation, and adjacent same-URI hyperlink merging.
- **Algorithms & Performance**:
  - Added 2D `SpatialGrid` for $O(1)$ average-case link attachment and table cell span assignment.
  - Added `UnionFind` (DSU) connected components to isolate multiple independent tables per page and cluster vector graphics.
- **Repository Structure & Python Package (`PyO3` + `maturin`)**:
  - Promoted `pymupdf4llm-rs` to repository root as a standalone pure-Rust library + CLI crate + Python wheel (`abi3-py39`).
  - Added reusable `PdfExtractor` and parse-once `ParsedPdf` Python classes holding shared `Arc<PdfEngine>`, `Arc<ToMarkdownOptions>`, Tokio runtime, global `ToUnicode` CMap cache, and `max_pdf_bytes` / `max_pages` safety guards with GIL release (`py.allow_threads`).
  - Enforced 64 MB decompression-bomb limits across `FlateDecode`, `LZWDecode`, `ASCII85Decode`, and `RunLengthDecode`.

### 简体中文
- **PDF 格式与文字编码兼容性扩展**：
  - 支持 `/Pages` 树向子页面继承 `/MediaBox`、`/CropBox`、`/Rotate` 与 `/Resources`。
  - 支持 `/Subtype /Form` 表单 XObject 递归解释执行及局部变换矩阵与字体栈叠加。
  - 实现纯 Rust 版 `LZWDecode` 解压器（支持 9–12 位可变码长与 `EarlyChange`）。
  - 完善 `Type0`/CID `/W` 宽度解析、数组型 `beginbfrange` `ToUnicode` CMap、`MacRomanEncoding` 与 70+ Adobe Glyph 字符名映射。
  - 增加 `/Rotate`（0°/90°/180°/270°）页面旋转坐标归一化与无锁 `DashMap` 字体缓存。
- **乱码治理与噪声过滤**：
  - 新增 `TextHygieneConfig` 与 `HygieneStats` 诊断统计（`clean_mojibake`、`dehyphenate`、`dedup_faux_bold`、`ignore_invisible_text`、`strip_watermarks`、`dedup_running_headers`）。
  - 自动过滤 `3 Tr` 隐形 OCR 层、斜向/低透明度水印、不可映射 PUA/控制符乱码以及伪粗体（Faux-Bold）重影。
  - 支持连字展开、PUA 项目符号归一化、换行断词拼接及相邻同 URI 超链接 Span 合并。
- **空间算法与性能优化**：
  - 引入二维 `SpatialGrid` 空间网格索引，将链接命中与表格单元匹配降至均摊 $O(1)$。
  - 引入并查集 `UnionFind` 连通分量算法，精准分离单页多张独立表格并高效聚类矢量图形。
- **仓库结构精简**：
  - 移除前端展示代码，将纯 Rust 工具包与 CLI 主程序置于仓库根目录。

---

## [0.1.0] - 2026-11-16

### English
- Initial pure-Rust `pymupdf4llm-rs` crate with PDF 1.0–2.0 parser, `FlateDecode`, recursive X-Y Cut multi-column ordering, `IdentifyHeaders` / `TocHeaders`, 4 table strategies, Tokio + Rayon concurrency, and CLI binary.

### 简体中文
- 初始版本发布：100% 纯 Rust `pymupdf4llm-rs` 核心库，支持 PDF 1.0–2.0 解析、`FlateDecode`、递归 X-Y Cut 多栏切分、`IdentifyHeaders`/`TocHeaders`、4 种表格策略、Tokio + Rayon 并发调度与 CLI 工具。
