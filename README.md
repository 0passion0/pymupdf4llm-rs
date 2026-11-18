# pymupdf4llm-rs

**简体中文** | [English](./README.en.md)

[![PyPI](https://img.shields.io/pypi/v/pymupdf4llm-rs?color=blue&label=PyPI)](https://pypi.org/project/pymupdf4llm-rs/)
[![Python](https://img.shields.io/pypi/pyversions/pymupdf4llm-rs)](https://pypi.org/project/pymupdf4llm-rs/)
[![CI](https://github.com/0passion0/pymupdf4llm-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/0passion0/pymupdf4llm-rs/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-green.svg)](./LICENSE)

**100% 纯 Rust 实现的高并发 PDF 转 Markdown 解析核心库（Python / Rust / CLI）**

`pymupdf4llm-rs` 是面向大语言模型（LLM）与检索增强生成（RAG）文档处理管线打造的零 C/C++ FFI、全纯 Rust 版 [`pymupdf4llm`](https://github.com/pymupdf/pymupdf4llm) 解析引擎。底层基于 **Tokio + Rayon** 提供有界高并发与工作窃取多核调度，在保持 `pymupdf4llm` 语义高度兼容的同时，消除了 MuPDF C 动态库分发体积大、GIL 阻塞与内存越界隐患。

---

## 目录

- [1. 核心特性与架构优势](#1-核心特性与架构优势)
- [2. 解析管线架构](#2-解析管线架构)
- [3. 安装指南（PyPI / GitHub / Cargo）](#3-安装指南pypi--github--cargo)
- [4. Python 快速上手与核心 API](#4-python-快速上手与核心-api)
- [5. Rust 库与 Web 高并发单例调用](#5-rust-库与-web-高并发单例调用)
- [6. 命令行工具（CLI）](#6-命令行工具cli)
- [7. 核心参数与安全边界说明](#7-核心参数与安全边界说明)
- [8. 仓库目录结构与开发测试](#8-仓库目录结构与开发测试)
- [9. 开源协议](#9-开源协议)

---

## 1. 核心特性与架构优势

| 能力维度 | `pymupdf4llm-rs` 纯 Rust 实现 | 传统 Python + C 绑定方案 |
|---|---|---|
| **二进制零依赖** | **100% 纯 Rust**，原生内置 PDF 1.0–2.0 二进制解析器与流解压器 | 依赖庞大 MuPDF / Poppler C 动态库，容器镜像膨胀且存在跨平台编译兼容痛点 |
| **Python GIL 释放** | 全解析与排版期间自动释放 GIL（`py.allow_threads`），支持真正的多线程并行 | 部分 C 回调与 Python 对象封装仍受 GIL 制约 |
| **单次解析多次复用** | `PdfExtractor` 持有共享引擎，`ParsedPdf` (`open()`) 单次解析导出多种格式 | 重复调用不同接口常触发重复文件 IO 与对象树解析 |
| **并发与防爆护栏** | 内置 `max_pdf_bytes`、64 MB 流解压炸弹拦截、`max_pages` 与 Tokio 信号量限流 | 遇到恶意畸形压缩流易引发 OOM 或线程资源耗尽 |
| **空间算法性能** | 二维 `SpatialGrid`（均摊 $O(1)$）+ 并查集 `UnionFind` 多表分离 | 稠密页面上常见 $O(N^2)$ 嵌套扫描瓶颈 |
| **乱码与噪声治理** | 内置连字还原、`3 Tr` 隐形 OCR 剥离、斜向水印/伪粗体过滤与跨页页眉去重 | 往往需要额外编写后处理正则脚本 |

---

## 2. 解析管线架构

```text
PDF 文件路径 / 内存字节 / ParsedPdf 句柄
  │
  ▼ [安全边界检查：max_pdf_bytes 大小拦截 & PDF Header 校验]
[1. 原生词法与对象树解析]  XRef / ObjStm / /Pages 属性继承 / 64MB 流解压限制 (Flate/LZW/A85/RLE)
  │
  ▼ [DashMap 无锁全局字体与 ToUnicode CMap 缓存]
[2. 内容流解释与字符解码]  Type0 CID /W 宽度 / beginbfchar & beginbfrange / /Subtype /Form 递归展开
  │
  ▼ [Rayon 逐页并行提取 + Tokio Semaphore 并发水位控制]
[3. 空间布局与版面还原]    SpatialGrid (O(1) 链接/表格单元匹配) · UnionFind 多表分离 · X-Y Cut 多栏排版
  │
  ▼ [TextHygieneConfig 文本净化]
[4. 噪声过滤与 Markdown 渲染] 排版连字还原 · 3 Tr 隐形字剥离 · 水印/伪粗体/页眉去重 · IdentifyHeaders/TocHeaders
  │
  ▼
Markdown 文本 / PageChunk 列表 / JSON / LlamaMarkdownReader 文档
```

---

## 3. 安装指南（PyPI / GitHub / Cargo）

### Python 安装（支持 Python `>= 3.9`，全平台 `abi3` 预编译 Wheel）

```bash
# 方式 1：直接从 PyPI 安装（推荐，自动匹配 Linux / macOS / Windows 预编译包）
pip install pymupdf4llm-rs

# 方式 2：从 GitHub 源码构建安装（需本地已配置 Rust 工具链）
pip install git+https://github.com/0passion0/pymupdf4llm-rs.git

# 方式 3：安装 GitHub Releases 预编译 Wheel（以 v0.4.2 Linux x86_64 为例）
pip install https://github.com/0passion0/pymupdf4llm-rs/releases/download/v0.4.2/pymupdf4llm_rs-0.4.2-cp39-abi3-manylinux2014_x86_64.manylinux_2_17_x86_64.whl
```

### Rust Crate / CLI 安装

```toml
# Cargo.toml
[dependencies]
pymupdf4llm-rs = "0.4.2"
tokio = { version = "1", features = ["full"] }
```

```bash
# 编译安装命令行工具
cargo install --path . --features cli
```

---

## 4. Python 快速上手与核心 API

### 4.1 推荐用法：复用 `PdfExtractor` 与单次解析 `ParsedPdf`

在 Web 后端（FastAPI / Flask）或批量 RAG 清洗任务中，建议在初始化阶段实例化一次 `PdfExtractor`，跨请求共享底层 `Arc<PdfEngine>` 资源池、Tokio 运行时、全局 `ToUnicode` 缓存及安全拦截上限：

```python
from pymupdf4llm_rs import LlamaMarkdownReader, PdfExtractor

# 1. 启动时实例化一次，持有共享引擎、并发信号量与安全阈值
extractor = PdfExtractor(
    table_strategy="lines_strict",
    clean_mojibake=True,
    strip_watermarks=True,
    ignore_invisible_text=True,
    dedup_faux_bold=True,
    max_concurrent_docs=4,
    max_concurrent_pages=8,
    max_pdf_bytes=64 * 1024 * 1024,  # 超过 64MB 在解析前直接拒绝
    max_pages=500,                   # 限制最大页数防止超大文档耗尽内存
)

# 2. 跨请求直接复用实例方法（解析阶段自动释放 Python GIL）
md_text = extractor.to_markdown("paper.pdf")
md_from_bytes = extractor.to_markdown_bytes(pdf_bytes)
chunks = extractor.to_chunks("paper.pdf", extract_words=True)

# 3. 派生轻量配置副本：无缝复用同一底层 Arc<PdfEngine> 与缓存
text_table_extractor = extractor.with_options(table_strategy="text", ignore_code=True)

# 4. 同一份 PDF 解析一次得到 ParsedPdf 句柄，零重复解析导出多种格式
with extractor.open(pdf_bytes) as doc:
    meta = doc.metadata()
    toc = doc.toc()
    md = doc.to_markdown()
    page_chunks = doc.to_chunks()
    json_str = doc.to_json()
    plain_text = doc.to_text()

# 5. 兼容 LlamaIndex 风格读取器（自动复用同一个 extractor）
reader = LlamaMarkdownReader(extractor=extractor)
llama_docs = reader.load_data("paper.pdf", extra_info={"source": "arxiv"})
```

### 4.2 顶层快捷函数

适合脚本与一次性任务：

```python
from pymupdf4llm_rs import to_chunks, to_json, to_markdown, to_markdown_bytes, to_text

md = to_markdown("paper.pdf", table_strategy="lines_strict", page_separators=True)
chunks = to_chunks("paper.pdf", extract_words=True)
json_str = to_json("paper.pdf")
plain = to_text("paper.pdf")
```

---

## 5. Rust 库与 Web 高并发单例调用

### 标准异步调用

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

### Web 服务全局单例 (`PdfEngine::global`)

在 Axum / Actix / Tonic 服务中处理高并发 PDF 上传请求时，使用进程级单例 `PdfEngine::global()` 统一限制全局在飞文档与页面并发数，并跨请求复用 `ToUnicode` CMap：

```rust
use pymupdf4llm_rs::{PdfEngine, ToMarkdownOptions};

let output = PdfEngine::global()
    .extract_bytes(pdf_bytes, ToMarkdownOptions::default())
    .await?;
```

---

## 6. 命令行工具（CLI）

```bash
# 提取 Markdown 并导出逐页 PageChunk JSON，指定单文档最大页面并发为 8
cargo run --release --features cli -- paper.pdf --page-chunks --concurrency 8 -o output.json

# 运行全部 Rust 单元测试与兼容性集成测试
cargo test --features "async,cli,images"
```

---

## 7. 核心参数与安全边界说明

| 参数名 | 类型 / 可选值 | 说明 |
|---|---|---|
| `table_strategy` | `"lines_strict"` \| `"lines"` \| `"text"` \| `"explicit"` | 表格检测策略：严格线段、线段扩展、纯文本对齐或显式表格边界 |
| `hdr_info` | `"identify"` \| `"toc"` \| `"disabled"` \| `None` | 标题识别模式：字号直方图聚类或 `/Outlines` 书签映射 |
| `page_chunks` | `bool` | 为 `True` 时返回含 `metadata`、`tables`、`words`、`hygiene` 的逐页字典列表 |
| `clean_mojibake` | `bool` | 开启连字展开、PUA 符号归一化与不可映射控制字符剥离 |
| `dehyphenate` | `bool` | 自动拼接英文/数字跨行断词连字符 |
| `dedup_faux_bold` | `bool` | 对重影叠加伪粗体（Faux-Bold）执行空间坐标去重 |
| `ignore_invisible_text` | `bool` | 过滤 `3 Tr` 隐形 OCR 文本层，防止与底层扫面文本重复 |
| `strip_watermarks` | `bool` | 过滤斜向或低透明度干扰水印文本 |
| `dedup_running_headers` | `bool` | 可选清理跨页重复出现的页眉与页脚 |
| `max_concurrent_docs` / `max_concurrent_pages` | `Optional[int]` | 分别限制同时解析的文档数与页面数并发上限 |
| `max_pdf_bytes` / `max_pages` | `int` / `Optional[int]` | 安全护栏：拒绝超大体积输入（默认 `128 MB`）及超限页数文档 |

---

## 8. 仓库目录结构与开发测试

```text
.
├── Cargo.toml                         # Rust Crate 配置与 Release 优化配置（LTO + codegen-units=1）
├── pyproject.toml                     # Maturin PEP 517 Python 扩展构建配置（abi3-py39）
├── LICENSE / CHANGELOG.md             # Apache-2.0 OR MIT 双重协议与版本变更记录
├── README.md / README.en.md           # 中文（默认）/ 英文双语文档
├── .github/workflows/
│   ├── ci.yml                         # rustfmt + clippy + cargo test + maturin + pytest 自动化检查
│   └── release-wheels.yml             # Linux(x64/ARM64) / macOS(Intel/M系列) / Windows 多平台自动发版
├── python/pymupdf4llm_rs/             # Python 封装入口、.pyi 类型存根与 LlamaMarkdownReader
├── src/
│   ├── lib.rs                         # 公共 API（to_markdown, to_json, to_text, MarkdownOutput）
│   ├── python.rs                      # PyO3 原生绑定（PdfExtractor, ParsedPdf, 释放 GIL）
│   ├── async_engine.rs                # Tokio 信号量限流 + Rayon 并行与全局 PdfEngine 单例
│   ├── config.rs                      # ToMarkdownOptions, TableStrategy, TextHygieneConfig
│   ├── text_cleaner.rs                # 乱码/PUA/3 Tr OCR/水印/伪粗体/跨页页眉清洗模块
│   ├── llama_reader.rs                # Rust 侧 LlamaMarkdownReader RAG 适配器
│   ├── pdf/                           # 二进制词法分析、Flate/LZW/A85/RLE（64MB 限流）与 CMap 缓存
│   ├── layout/                        # SpatialGrid、UnionFind 多表分离、X-Y Cut 多栏切分、标题聚类
│   ├── markdown/                      # PageChunk 与 Markdown 渲染、断字拼接、相邻链接合并
│   └── bin/pymupdf4llm.rs             # CLI 命令行入口
└── tests/
    ├── compatibility_tests.rs         # Rust 集成测试（多栏、CJK、Form XObject、清洗、异步并发）
    └── test_python_api.py             # Pytest 集成测试（实例复用、ParsedPdf、安全边界、多线程 GIL）
```

本地源码构建与完整校验命令：

```bash
# Python 扩展本地编译与测试
pip install maturin pytest
maturin develop --release
pytest -v tests/test_python_api.py

# Rust 格式、Clippy 静态检查、文档生成与集成测试
cargo fmt --all -- --check
cargo clippy --all-targets --features "async,cli,images,python" -- -D warnings
cargo test --features "async,cli,images"
cargo doc --no-deps --features "async,cli,images"
```

---

## 9. 开源协议

本项目采用 **[Apache-2.0 OR MIT](LICENSE)** 双重协议开源。
