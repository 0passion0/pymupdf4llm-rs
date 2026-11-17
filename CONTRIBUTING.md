# Contributing to pymupdf4llm-rs / 参与贡献指南

Thank you for contributing to `pymupdf4llm-rs`! Bug reports, PDF compatibility fixtures, performance optimizations, and documentation improvements are welcome in both **English** and **简体中文**.

感谢您关注并参与 `pymupdf4llm-rs`！欢迎提交 Bug 反馈、疑难 PDF 兼容性测试用例、性能优化以及文档完善（支持**英文**或**简体中文**）。

---

## 1. Development Setup / 本地开发环境

- Rust `1.78+` (Edition 2021)
- 100% pure-Rust dependencies only (`no C/C++ FFI`) / 仅允许纯 Rust 依赖，严禁引入 C/C++ FFI

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

---

## 2. Coding Guidelines / 开发规范

1. **Zero C/C++ FFI / 零外部原生依赖**：Keep all parsing, decompression, and layout code in safe pure Rust.
2. **Algorithmic Complexity / 算法复杂度控制**：Use `SpatialGrid` and `UnionFind` to avoid $O(N^2)$ scans on dense PDF pages.
3. **Tests / 补充测试**：Add unit or integration tests in `tests/compatibility_tests.rs` for any new PDF syntax, font encoding, or hygiene rule.

---

## 3. Commit Convention / 提交信息规范

We follow [Conventional Commits](https://www.conventionalcommits.org/):

- `feat(pdf): add RunLengthDecode stream filter / 新增 RunLengthDecode 流解压`
- `fix(hygiene): preserve compound hyphenated terms across line wraps / 修复跨行复合词断字拼接误伤`
- `perf(layout): optimize SpatialGrid cell sizing / 优化 SpatialGrid 网格粒度`
- `docs: update README.zh-CN.md / 更新中文文档`

---

## 4. Pull Request Checklist / PR 自检清单

- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test --all-features` passes.
- [ ] Bilingual docs (`README.md` / `README.zh-CN.md` / `CHANGELOG.md`) updated if public API changed.
