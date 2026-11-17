# Contributing to pymupdf4llm-rs / 参与贡献指南

Thank you for contributing to `pymupdf4llm-rs`! We welcome bug reports, PDF compatibility fixtures, performance optimizations, and documentation improvements in both **English** and **简体中文**.

感谢您关注并参与 `pymupdf4llm-rs`！我们非常欢迎提交 Bug 反馈、疑难 PDF 兼容性测试用例、性能优化以及文档完善（支持**英文**或**简体中文**）。

---

## 1. Development Setup / 本地开发环境

### Rust Crate (`rust/pymupdf4llm-rs`)
- Rust `1.78+` (Edition 2021)
- Pure Rust dependencies only (`no C/C++ FFI`)

```bash
cd rust/pymupdf4llm-rs
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

### Web Workbench & TypeScript Mirror (`src/`)
- Node.js `20+`

```bash
npm install
npm run lint
npx tsx --test src/engine/pdfEngine.test.ts
npm run build
```

---

## 2. Architecture Parity Rules / 双端同步规范

When modifying parsing, layout, or hygiene logic, keep the **Rust crate** (`rust/pymupdf4llm-rs/src/`) and **TypeScript mirror** (`src/engine/`) behaviorally aligned:
修改解析、版面还原或文本清洗逻辑时，请保持 **Rust 核心库** 与 **TypeScript 同构内核** 的行为一致性：

1. **Zero C/C++ Dependencies / 严禁引入 C/C++ 依赖**：All Rust crates must remain pure Rust.
2. **Algorithmic Complexity / 算法复杂度控制**：Avoid $O(N^2)$ scans over spans or vector segments; use `SpatialGrid` and `UnionFind`.
3. **Unit & Integration Tests / 补充测试**：Add corresponding test cases in both `rust/pymupdf4llm-rs/tests/` and `src/engine/pdfEngine.test.ts`.

---

## 3. Commit Message Convention / 提交信息规范

We follow [Conventional Commits](https://www.conventionalcommits.org/):
请遵循 Conventional Commits 提交格式：

- `feat(pdf): add RunLengthDecode stream filter / 新增 RunLengthDecode 流解压`
- `fix(hygiene): preserve compound hyphenated terms across line wraps / 修复跨行复合词断字拼接误伤`
- `perf(layout): optimize SpatialGrid cell sizing / 优化 SpatialGrid 网格粒度`
- `docs: update README.zh-CN.md and CONTRIBUTING.md / 更新中英文文档`
- `test(compat): add rotated CropBox integration fixture / 新增旋转裁剪框集成测试`

---

## 4. Pull Request Checklist / PR 自检清单

- [ ] `cargo fmt`, `cargo clippy`, and `cargo test --all-features` pass (for Rust changes).
- [ ] `npm run lint`, `npx tsx --test src/engine/pdfEngine.test.ts`, and `npm run build` pass (for Web/TS changes).
- [ ] No hardcoded secrets, local `.env` files, or proprietary copyrighted PDFs committed.
- [ ] Bilingual docs (`README.md` / `README.zh-CN.md` / `CHANGELOG.md`) updated if public API or options changed.
