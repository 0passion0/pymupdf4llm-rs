# Security Policy / 安全政策

## Supported Versions / 受支持版本

| Version / 版本 | Supported / 支持状态 |
| --- | --- |
| `0.4.x` | Supported / 持续维护 |
| `< 0.4.0` | Upgrade recommended / 建议升级至最新版 |

---

## Reporting a Vulnerability / 报告安全漏洞

### English
Because `pymupdf4llm-rs` parses untrusted binary PDF streams (`XRef`, `ObjStm`, `FlateDecode`, `LZWDecode`, `Form` XObjects, and `ToUnicode` CMaps), please **do not open a public GitHub issue** for security-sensitive bugs such as:
- Infinite loops, stack overflows, or unbounded recursion on malformed PDFs (DoS / OOM).
- Decompression bombs (`FlateDecode` / `LZWDecode` amplification).
- Path traversal when `write_images = true` and `image_path` is configured.

Instead, report vulnerabilities privately via GitHub **Security Advisories** ("Report a vulnerability" in the Security tab) with:
1. Affected version (`Cargo.toml` / `package.json`).
2. Minimal sanitized reproduction PDF or byte sequence.
3. Expected vs. observed behavior.

### 简体中文
由于 `pymupdf4llm-rs` 需要直接解析外部不可信的 PDF 二进制流（如 `XRef`、`ObjStm`、`FlateDecode`、`LZWDecode`、`Form` XObject 递归及 `ToUnicode` CMap），若发现以下安全隐患，请**勿直接在公开 Issue 中披露细节**：
- 畸形 PDF 导致的死循环、栈溢出、深度递归或内存耗尽（DoS / OOM）。
- 解压缩炸弹（`FlateDecode` / `LZWDecode` 膨胀攻击）。
- 开启 `write_images = true` 时的路径穿越风险。

请通过仓库 **Security** 标签页中的 **Security Advisories（私密安全公告）** 联系维护者，并提供：
1. 受影响的版本号。
2. 脱敏后的最小复现 PDF 或字节片段。
3. 预期行为与实际异常表现。
