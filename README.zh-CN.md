# pymupdf4llm-rs

[English](README.md) | [简体中文](README.zh-CN.md)

**100% 纯 Rust 实现的 `pymupdf4llm` PDF 转 Markdown 核心库与命令行工具**

`pymupdf4llm-rs` 是面向大模型（LLM）与检索增强生成（RAG）文档解析场景打造的零 C/C++ 绑定、全纯 Rust 版 [`pymupdf4llm`](https://github.com/pymupdf/pymupdf4llm) 工具库，基于 Tokio + Rayon 提供高并发异步解析能力。

---

## 核心特性

- **100% 纯 Rust 实现（无 C/C++ FFI）**：内置原生 PDF 1.0–2.0 二进制解析器（支持 `XRef`、`ObjStm`、`/Pages` 树属性继承、`/Subtype /Form` XObject 递归展开、`FlateDecode`、`LZWDecode` 与 `ASCIIHexDecode`），仅依赖纯 Rust `flate2`（`rust_backend`）。
- **异步与高并发调度**：
  - 全文档仅解析一次，通过零拷贝 `Arc<PdfDocument>` 在工作线程间共享。
  - 使用 `tokio::sync::Semaphore` 限制并发水位，结合 `rayon` 工作窃取线程池并行执行逐页提取与批量文档处理（`to_markdown_async`、`to_markdown_stream`、`batch_to_markdown_async`）。
  - 采用无锁 `DashMap` 全局缓存字体编码与 `ToUnicode` CMap。
- **100% 兼容 `pymupdf4llm` 接口与输出**：
  - 完整支持 `to_markdown()`、`to_markdown_with_options()`、`to_json()`、`to_text()` 与 `LlamaMarkdownReader`。
  - `page_chunks = true` 生成完整 `PageChunk`（`metadata`、`toc_items`、`tables`、`images`、`graphics`、`words`、`text`、`hygiene`）。
  - 支持 `IdentifyHeaders`（字号直方图聚类）与 `TocHeaders`（`/Outlines` 书签映射）。
  - 完整实现 4 种表格策略（`LinesStrict`、`Lines`、`Text`、`Explicit`）、递归 X-Y Cut 多栏排版、代码块、列表与 `page_separators`。
- **深度乱码治理与高性能空间算法（`v0.4.0`）**：
  - **二维 `SpatialGrid` 空间网格**：将链接匹配与表格单元查询复杂度降至均摊 $O(1)$。
  - **并查集 `UnionFind`（路径压缩 + 按秩合并）**：单页精准分离多张独立表格并在 $O(N \cdot \alpha(N))$ 时间内聚类矢量图形。
  - **更广字体与 CJK 编码兼容**：支持 `Type0`/CID `/W` 宽度、`beginbfchar` 与数组型 `beginbfrange` `ToUnicode` CMap、`MacRomanEncoding`、`WinAnsiEncoding`、70+ Adobe Glyph 字符名映射及 `/Rotate` 页面旋转归一化。
  - **乱码与噪声过滤（`TextHygieneConfig`）**：排版连字还原（`ﬀ`/`ﬁ`/`ﬂ`/`ﬃ`/`ﬄ`/`ﬅ`）、Wingdings PUA 符号归一化、不可映射 PUA 与控制字符剥离、行尾断字拼接、过滤 `3 Tr` 隐形 OCR 层、过滤斜向/半透明水印、伪粗体（Faux-Bold）去重，以及可选跨页重复页眉/页脚清理。

---

## 快速开始

### 作为 Rust 库调用

```toml
[dependencies]
pymupdf4llm-rs = { path = "." }
tokio = { version = "1", features = ["full"] }
```

```rust
use pymupdf4llm_rs::{
    to_markdown_async, MarkdownOutput, TableStrategy, ToMarkdownOptions,
};

#[tokio::main]
async fn main() -> pymupdf4llm_rs::Result<()> {
    let opts = ToMarkdownOptions::builder()
        .page_chunks(true)
        .table_strategy(TableStrategy::LinesStrict)
        .extract_words(true)
        .max_page_concurrency(8)
        .build();

    match to_markdown_async("paper.pdf", opts).await? {
        MarkdownOutput::Chunks(chunks) => {
            for chunk in chunks {
                println!(
                    "第 {} 页: {} 字符, {} 张表格",
                    chunk.metadata.page,
                    chunk.text.len(),
                    chunk.tables.len()
                );
            }
        }
        MarkdownOutput::Text(md) => println!("{md}"),
    }
    Ok(())
}
```

### Web 服务高并发全局单例调用（`PdfEngine::global`）

在 Web 后端（Axum / Actix / Tonic）处理高并发 PDF 上传时，建议使用进程级全局单例 `PdfEngine::global()` 统一限流全进程内的文档与页面并发度，并自动跨请求安全复用相同内容哈希的 `ToUnicode` CMap：

```rust
use pymupdf4llm_rs::{PdfEngine, ToMarkdownOptions};

let output = PdfEngine::global()
    .extract_bytes(pdf_bytes, ToMarkdownOptions::default())
    .await?;
```

### Python 包安装与调用（支持直接 `pip install` GitHub 仓库）

其他 Python（`>= 3.9`）项目可直接通过 `pip` 安装使用（若部署环境无 Rust 编译器，可在 GitHub Actions 发布 Tag 后直接安装 Release 预编译 `.whl` 二进制包）：

```bash
pip install git+https://github.com/0passion0/pymupdf4llm-rs.git
# 或写入 requirements.txt / pyproject.toml：
# pymupdf4llm-rs @ git+https://github.com/0passion0/pymupdf4llm-rs.git@main
```

本地源码开发测试：

```bash
pip install maturin pytest
maturin develop --release
pytest -v tests/test_python_api.py
```

在应用启动时实例化一次 `PdfExtractor`，内部会持久持有共享的 Rust 资源（`Arc<PdfEngine>` 并发信号量、Tokio + Rayon 线程池、全局 `ToUnicode` CMap 缓存及 `max_pdf_bytes` / `max_pages` 安全上限），随后跨请求高频复用实例下的各解析方法与默认参数：

```python
from pymupdf4llm_rs import PdfExtractor

# 1. 启动时实例化一次，持有共享引擎与参数/安全上限
extractor = PdfExtractor(
    table_strategy="lines_strict",
    clean_mojibake=True,
    strip_watermarks=True,
    ignore_invisible_text=True,
    max_concurrent_docs=4,
    max_concurrent_pages=8,
    max_pdf_bytes=64 * 1024 * 1024,  # 超过体积限制在解析前直接拦截
    max_pages=500,                   # 限制最大页数防止超大 PDF 耗尽内存
)

# 2. 跨请求直接复用实例方法（解析期间自动释放 Python GIL）
md = extractor.to_markdown("paper.pdf")
md_from_bytes = extractor.to_markdown_bytes(pdf_bytes)
chunks = extractor.to_chunks("paper.pdf", extract_words=True)

# 3. 派生轻量配置副本：共享同一个底层 Arc<PdfEngine> 资源池
text_table_extractor = extractor.with_options(table_strategy="text", ignore_code=True)

# 4. 同一份 PDF 解析一次得到 ParsedPdf 句柄，零重复解析导出多种格式
with extractor.open(pdf_bytes) as doc:
    meta = doc.metadata()
    toc = doc.toc()
    md = doc.to_markdown()
    chunks = doc.to_chunks()
    json_str = doc.to_json()
    plain = doc.to_text()
```

### 命令行工具（CLI）调用

```bash
cargo run --release --features cli -- paper.pdf --page-chunks --concurrency 8 -o output.json
cargo test --features "async,cli,images"
```

---

## 目录结构

```text
.
├── Cargo.toml
├── pyproject.toml                 # Maturin Python 扩展构建配置（abi3-py39）
├── python/pymupdf4llm_rs/         # Python 封装入口、.pyi 类型存根与 LlamaMarkdownReader
├── src/
│   ├── lib.rs                     # 公共 API (to_markdown, to_json, to_text, PageChunk)
│   ├── python.rs                  # PyO3 原生绑定（PdfExtractor, ParsedPdf, 释放 GIL）
│   ├── async_engine.rs            # Tokio 信号量限流 + Rayon 并行与全局 PdfEngine
│   ├── config.rs                  # ToMarkdownOptions, TableStrategy, TextHygieneConfig
│   ├── text_cleaner.rs            # 乱码/PUA/3 Tr OCR/水印/伪粗体/跨页页眉清洗模块
│   ├── llama_reader.rs            # LlamaMarkdownReader RAG 接口
│   ├── pdf/                       # 二进制词法分析、Flate/LZW/A85/RLE（64MB 限流）与 CMap 缓存
│   ├── layout/                    # SpatialGrid、UnionFind 多表分离、X-Y Cut 多栏切分、标题聚类
│   ├── markdown/                  # PageChunk 与 Markdown 渲染、断字拼接、相邻链接合并
│   └── bin/pymupdf4llm.rs         # CLI 命令行入口
└── tests/
    ├── compatibility_tests.rs     # Rust 集成测试（多栏、CJK、Form XObject、清洗、异步并发）
    └── test_python_api.py         # Pytest 集成测试（实例复用、ParsedPdf、安全边界、多线程 GIL）
```

---

## 开源协议

本项目采用 [Apache-2.0 OR MIT](LICENSE) 双重协议开源。
