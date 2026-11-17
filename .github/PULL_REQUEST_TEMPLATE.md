## Summary / 变更说明

<!-- Summarize what changed in English and/or 简体中文 -->
- 

## Type of Change / 变更类型

- [ ] PDF Format / Encoding Compatibility (`XRef`, `LZWDecode`, `ToUnicode`, `Form XObject`, etc.)
- [ ] Text Hygiene & Noise Cleanup (`mojibake`, `3 Tr OCR`, watermarks, de-hyphenation, running headers)
- [ ] Layout & Algorithms (`SpatialGrid`, `UnionFind`, X-Y Cut, header clustering)
- [ ] Async & Concurrency (`Tokio`, `Rayon`, `DashMap`, streaming)
- [ ] Web Workbench / Documentation (`README`, CI, UI)

## Verification Checklist / 验证清单

- [ ] `cargo fmt --all -- --check` & `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test --all-features`
- [ ] `npm run lint` & `npx tsx --test src/engine/pdfEngine.test.ts`
- [ ] `npm run build`
- [ ] Updated bilingual docs (`README.md` / `README.zh-CN.md` / `CHANGELOG.md`) if needed
