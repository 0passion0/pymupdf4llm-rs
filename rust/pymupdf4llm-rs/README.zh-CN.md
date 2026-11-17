# pymupdf4llm-rs

[English](README.md) | [简体中文](README.zh-CN.md)

100% 纯 Rust 实现的 [pymupdf4llm](https://github.com/pymupdf/pymupdf4llm) 工具库，完全兼容其核心 API 与 Markdown/Chunk 输出结构，内置乱码/噪点治理管线、`SpatialGrid` + `UnionFind` 空间版面算法，以及基于 Tokio + Rayon 的异步高并发调度引擎。

## 核心能力

- **100% 纯 Rust（零 C/C++ FFI）**：仅依赖纯 Rust `flate2`（`rust_backend`），原生实现 PDF 1.0–2.0 `XRef`/`ObjStm` 解析、`/Pages` 树 `/MediaBox` 与 `/Resources` 属性继承、`/Subtype /Form` 表单 XObject 递归展开、`FlateDecode` + `LZWDecode` 解压、`Type0`/CID `/W` 宽度解析、`ToUnicode` CMap（`beginbfchar` + 数组型 `beginbfrange`）、`MacRomanEncoding`、70+ Adobe Glyph 名称映射与 `/Rotate` 旋转归一化。
- **全面兼容 `pymupdf4llm`**：
  - `to_markdown()` 与 `to_markdown_with_options()`
  - `page_chunks = true` 生成完整 `PageChunk` 结构（`metadata`、`toc_items`、`tables`、`images`、`graphics`、`words`、`text`、`hygiene`）
  - `IdentifyHeaders`（字号直方图聚类）与 `TocHeaders`（`/Outlines` 书签映射）
  - 支持 4 种表格策略：`LinesStrict`、`Lines`、`Text`、`Explicit`
  - 递归 X-Y Cut 多栏阅读顺序 + `SpatialGrid` 均摊 $O(1)$ 链接/单元检索 + `UnionFind` 单页多表分离与矢量图形聚类
  - 粗体/斜体/删除线/代码块（`ignore_code`）、同 URI 相邻超链接自动合并、列表、引用与 `page_separators`
  - 图像提取与矢量聚类（`write_images`、`embed_images`、`image_path`、`image_format`、`dpi`、`image_size_limit`、`graphics_limit`）
  - 兼容 RAG / LlamaIndex 场景的 `LlamaMarkdownReader`
- **文本清洗与噪声过滤（`TextHygieneConfig`）**：
  - 排版连字还原（`ﬀ`/`ﬁ`/`ﬂ`/`ﬃ`/`ﬄ`/`ﬅ`）、Wingdings PUA 符号归一化、不可映射 PUA 与控制字符剥离
  - 行尾连字符断词智能拼接（`dehyphenate`）
  - 过滤扫描件叠加的 `3 Tr` 隐形 OCR 层（`ignore_invisible_text`）
  - 过滤斜向旋转与半透明水印（`strip_watermarks`）
  - 伪粗体（Faux-Bold）重影去重（`dedup_faux_bold`）
  - 可选跨页重复页眉/页脚自动剔除（`dedup_running_headers`）
- **异步与高并发**：
  - 通过 `tokio::sync::Semaphore` 控制页面并发度，配合 `rayon` 工作窃取线程池高效并行解析
  - 支持基于 `futures::Stream` 的逐页流式输出 `to_markdown_stream()`
  - 零拷贝 `Arc<PdfDocument>` 配合无锁 `DashMap` 字体缓存跨线程共享

## 快速上手

```rust
use pymupdf4llm_rs::{
    to_markdown_async, MarkdownOutput, TableStrategy, ToMarkdownOptions,
};

#[tokio::main]
async fn main() -> pymupdf4llm_rs::Result<()> {
    let opts = ToMarkdownOptions::builder()
        .page_chunks(true)
        .table_strategy(TableStrategy::LinesStrict)
        .write_images(false)
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
