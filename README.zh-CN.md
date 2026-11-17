# pymupdf4llm-rs

[English](README.md) | [简体中文](README.zh-CN.md)

**100% 纯 Rust 实现的 `pymupdf4llm` PDF 转 Markdown 引擎与交互式兼容工作台**

`pymupdf4llm-rs` 是面向大模型（LLM）与检索增强生成（RAG）文档入库场景打造的零 C/C++ 绑定、全纯 Rust 版 [`pymupdf4llm`](https://github.com/pymupdf/pymupdf4llm) 引擎，并配套提供基于浏览器和 Express/Vite 的实时解析调试与对比工作台。

---

## 核心特性

- **100% 纯 Rust 实现（无 C/C++ FFI）**：内置原生 PDF 1.0–2.0 二进制解析器（支持 `XRef`、`ObjStm`、`/Pages` 树属性继承、`/Subtype /Form` XObject 递归展开、`FlateDecode`、`LZWDecode` 与 `ASCIIHexDecode`），底层仅依赖纯 Rust `flate2`（`rust_backend`）。
- **异步与高并发调度**：
  - 全文档仅解析一次，通过零拷贝 `Arc<PdfDocument>` 在工作线程间共享。
  - 使用 `tokio::sync::Semaphore` 限制并发水位，结合 `rayon` 工作窃取线程池并行执行逐页提取与批量文档处理（`to_markdown_async`、`to_markdown_stream`、`batch_to_markdown_async`）。
  - 采用无锁 `DashMap` 全局缓存字体编码与 `ToUnicode` CMap，避免高并发场景下的重复字典解析开销。
- **100% 兼容 `pymupdf4llm` 参数与输出结构**：
  - 完整支持 `to_markdown()`、`to_markdown_with_options()` 与 `PageChunk` 结构（`metadata`、`toc_items`、`tables`、`images`、`graphics`、`words`、`text`、`hygiene`）。
  - 支持 `IdentifyHeaders`（按字符数加权的字号直方图聚类）与 `TocHeaders`（`/Outlines` 书签映射）。
  - 完整实现 4 种表格识别策略：`LinesStrict`、`Lines`、`Text`、`Explicit`。
  - 递归 X-Y Cut 多栏阅读顺序还原、代码块围栏、多级列表、相邻超链接合并与 `LlamaMarkdownReader`。
- **深度乱码治理与高性能空间算法（`v0.4.0`）**：
  - **二维 `SpatialGrid` 空间哈希网格**：将超链接匹配、表格单元格归属与区间检索的平均复杂度从 $O(N \cdot M)$ 优化至均摊 $O(1)$。
  - **并查集 `UnionFind`（路径压缩 + 按秩合并）**：在同一页面内精准隔离多张独立表格，并在 $O(N \cdot \alpha(N))$ 时间内聚类矢量图形区域。
  - **更广字体与 CJK 编码兼容**：支持 `Type0`/CID 双字节编码与 `/W` 宽度表、`beginbfchar` / 数组型 `beginbfrange` `ToUnicode` CMap、`MacRomanEncoding`、`WinAnsiEncoding`、70+ Adobe Glyph 名称映射及 `/Rotate` 页面旋转归一化。
  - **乱码与噪声自动清洗**：排版连字还原（`ﬀ`/`ﬁ`/`ﬂ`/`ﬃ`/`ﬄ`/`ﬅ`）、Wingdings/Symbol 私有区（PUA）符号归一化、不可映射 PUA 与控制字符剥离、行尾断字智能拼接、过滤 `3 Tr` 扫描件隐形 OCR 文本层、过滤斜向/半透明水印、伪粗体（Faux-Bold）重影去重，以及可选的跨页重复页眉/页脚清理。

---

## 快速开始

### 1. Rust Crate（`rust/pymupdf4llm-rs`）

```toml
[dependencies]
pymupdf4llm-rs = { path = "./rust/pymupdf4llm-rs" }
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

运行 Rust 测试与 CLI：

```bash
cd rust/pymupdf4llm-rs
cargo test --all-features
cargo run --release --features cli -- ../../sample.pdf --page-chunks --concurrency 8
```

### 2. 交互式 Web 工作台（Express + Vite + React）

```bash
npm install
npm run dev
```

浏览器访问 `http://localhost:3000`，即可在线上传 PDF 或切换合成压测样例，实时验证多栏排版、文本清洗开关、诊断计数、`Page Chunks JSON`，并浏览或打包下载完整 Rust Crate 源码。

运行 TypeScript 内核校验与单元测试：

```bash
npm run lint
npx tsx --test src/engine/pdfEngine.test.ts
npm run build
```

---

## 仓库目录结构

```text
.
├── .github/                  # GitHub Actions CI 工作流、Issue 表单与 PR 模板（中英双语）
├── rust/pymupdf4llm-rs/      # 100% 纯 Rust 核心库（Tokio + Rayon + SpatialGrid + UnionFind）
│   ├── src/
│   │   ├── pdf/              # PDF 二进制词法分析、Flate/LZW 解码、字体/ToUnicode、内容流状态机
│   │   ├── layout/           # SpatialGrid、UnionFind 多表聚类、X-Y Cut 多栏切分、标题识别
│   │   ├── markdown/         # PageChunk 与 Markdown 渲染、断字拼接、相邻链接合并
│   │   ├── text_cleaner.rs   # 乱码/PUA/3 Tr OCR/水印/伪粗体/跨页页眉清洗模块
│   │   ├── async_engine.rs   # Tokio 信号量限流 + Rayon 并行与流式输出
│   │   └── bin/              # 命令行工具 (`pymupdf4llm`)
│   └── tests/                # 兼容性与清洗管线集成测试
├── src/
│   ├── engine/               # TypeScript 同构解析内核、合成 PDF 夹具与单元测试
│   └── components/           # 工作台、异步流水线视图、Rust 源码浏览器与参数兼容矩阵
├── server.ts                 # Express + Vite 服务及 `/api/crate/download` 打包接口
├── README.md                 # 英文说明文档
├── README.zh-CN.md           # 简体中文说明文档
├── CONTRIBUTING.md           # 中英双语贡献指南
├── CHANGELOG.md              # 中英双语版本更新日志
├── SECURITY.md               # 中英双语安全漏洞报告政策
└── LICENSE                   # Apache-2.0 OR MIT 双重开源协议
```

---

## 开源协议

本项目采用 [Apache-2.0 OR MIT](LICENSE) 双重协议开源。
