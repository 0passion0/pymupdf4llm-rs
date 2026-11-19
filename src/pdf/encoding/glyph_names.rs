use super::tables::{adobe_standard_high_to_str, symbol_code_to_str};

/// Decodes an Adobe / Type1C / Advent / Dingbats glyph name into Unicode.
pub fn glyph_name_to_unicode(name: &str, base_font: &str) -> Option<String> {
    if let Some(s) = static_glyph_name_to_unicode(name) {
        return Some(s.to_string());
    }

    // Single-character ASCII glyph names (e.g. /A, /a, /0)
    if name.len() == 1 {
        let b = name.as_bytes()[0];
        if b.is_ascii_alphanumeric() {
            return Some((b as char).to_string());
        }
    }

    // Adobe Glyph List `uniXXXX` (4-hex-digit sequences) and `uXXXXX` scalar syntax
    if let Some(hex) = name.strip_prefix("uni") {
        if hex.len() >= 4 && hex.len() % 4 == 0 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            let mut units = Vec::with_capacity(hex.len() / 4);
            let mut valid = true;
            for chunk in hex.as_bytes().chunks_exact(4) {
                if let Ok(s) = std::str::from_utf8(chunk) {
                    if let Ok(u) = u16::from_str_radix(s, 16) {
                        units.push(u);
                        continue;
                    }
                }
                valid = false;
                break;
            }
            if valid {
                let decoded = String::from_utf16_lossy(&units);
                if !decoded.is_empty() && !decoded.chars().all(|c| c.is_control()) {
                    return Some(decoded);
                }
            }
        }
    }
    if let Some(hex) = name.strip_prefix('u') {
        if (4..=6).contains(&hex.len()) && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            if let Ok(cp) = u32::from_str_radix(hex, 16) {
                if cp >= 0x20 {
                    if let Some(ch) = char::from_u32(cp) {
                        if !ch.is_control() {
                            return Some(ch.to_string());
                        }
                    }
                }
            }
        }
    }

    // Embedded Type1C / Advent 3B2 / TeX subset `/C<decimal>` glyph names
    if let Some(num_str) = name.strip_prefix('C') {
        if !num_str.is_empty() && num_str.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(code) = num_str.parse::<u32>() {
                return decode_numeric_c_glyph(code, base_font);
            }
        }
    }

    // ZapfDingbats `/a<decimal>` glyph names (checkmarks, bullets, stars, arrows)
    if let Some(num_str) = name.strip_prefix('a') {
        if !num_str.is_empty() && num_str.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(code) = num_str.parse::<u32>() {
                if let Some(ch) = zapf_dingbats_a_glyph(code) {
                    return Some(ch.to_string());
                }
            }
        }
    }

    // Variant suffix stripping per Adobe Glyph List spec (e.g. `fi.liga`, `A.sc`)
    if !name.starts_with('.') {
        if let Some((base, _variant)) = name.split_once('.') {
            if !base.is_empty() {
                return glyph_name_to_unicode(base, base_font);
            }
        }
    }

    // Compound ligature names separated by `_` (e.g. `T_h`, `uni0066_uni0069`)
    if name.contains('_') && !name.starts_with('_') && !name.ends_with('_') {
        let mut combined = String::new();
        let mut all_ok = true;
        for part in name.split('_') {
            if part.is_empty() {
                all_ok = false;
                break;
            }
            if let Some(sub) = glyph_name_to_unicode(part, base_font) {
                combined.push_str(&sub);
            } else {
                all_ok = false;
                break;
            }
        }
        if all_ok && !combined.is_empty() {
            return Some(combined);
        }
    }

    None
}

fn decode_numeric_c_glyph(code: u32, base_font: &str) -> Option<String> {
    // Font names with `+<HH>` high-byte Unicode page suffix (e.g. `AdvTT5843c571+20` + `/C19` -> U+2013 '–')
    if code < 256 {
        if let Some((_, suffix)) = base_font.rsplit_once('+') {
            if suffix.len() == 2 && suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
                if let Ok(hi) = u32::from_str_radix(suffix, 16) {
                    if hi > 0 {
                        if let Some(ch) = char::from_u32((hi << 8) | code) {
                            if !ch.is_control() {
                                return Some(ch.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    let lower = base_font.to_ascii_lowercase();
    if lower.contains("advpi") || lower == "symbol" || lower.ends_with("+symbol") {
        if let Some(sym) = symbol_code_to_str(code) {
            return Some(sym.to_string());
        }
    }
    if lower.contains("advt001") && code == 223 {
        return Some("©".to_string());
    }

    if (32..=126).contains(&code) {
        return Some((code as u8 as char).to_string());
    }

    if let Some(std_ch) = adobe_standard_high_to_str(code) {
        return Some(std_ch.to_string());
    }

    if code >= 256 {
        if let Some(ch) = char::from_u32(code) {
            if !ch.is_control() {
                return Some(ch.to_string());
            }
        }
    }

    None
}

fn zapf_dingbats_a_glyph(code: u32) -> Option<&'static str> {
    match code {
        1 => Some("✁"),
        2 => Some("✂"),
        3 => Some("✃"),
        4 => Some("✄"),
        19 => Some("✓"),
        20 => Some("✔"),
        21 => Some("✕"),
        22 => Some("✖"),
        23 => Some("✗"),
        24 => Some("✘"),
        35 => Some("★"),
        36 => Some("☆"),
        71 => Some("●"),
        72 => Some("❍"),
        73 => Some("■"),
        74 => Some("❏"),
        75 => Some("❐"),
        76 => Some("❑"),
        77 => Some("❒"),
        108 => Some("●"),
        109 => Some("❍"),
        110 => Some("■"),
        160 => Some("→"),
        161 => Some("➔"),
        165 => Some("➜"),
        _ => None,
    }
}

fn static_glyph_name_to_unicode(name: &str) -> Option<&'static str> {
    match name {
        "space" | "nbspace" | "nonbreakingspace" => Some(" "),
        "exclam" => Some("!"),
        "exclamdown" => Some("¡"),
        "quotedbl" => Some("\""),
        "numbersign" => Some("#"),
        "dollar" => Some("$"),
        "percent" => Some("%"),
        "ampersand" => Some("&"),
        "quotesingle" => Some("'"),
        "parenleft" => Some("("),
        "parenright" => Some(")"),
        "asterisk" => Some("*"),
        "plus" => Some("+"),
        "comma" => Some(","),
        "hyphen" | "minus" | "sfthyphen" | "hyphensoft" | "nonbreakinghyphen" => Some("-"),
        "period" => Some("."),
        "slash" | "fraction" => Some("/"),
        "zero" | "zerooldstyle" => Some("0"),
        "one" | "oneoldstyle" => Some("1"),
        "two" | "twooldstyle" => Some("2"),
        "three" | "threeoldstyle" => Some("3"),
        "four" | "fourextra" | "fouroldstyle" => Some("4"),
        "five" | "fiveoldstyle" => Some("5"),
        "six" | "sixoldstyle" => Some("6"),
        "seven" | "sevenoldstyle" => Some("7"),
        "eight" | "eightoldstyle" => Some("8"),
        "nine" | "nineoldstyle" => Some("9"),
        "colon" => Some(":"),
        "semicolon" => Some(";"),
        "less" => Some("<"),
        "equal" => Some("="),
        "greater" => Some(">"),
        "question" => Some("?"),
        "questiondown" => Some("¿"),
        "at" => Some("@"),
        "bracketleft" => Some("["),
        "backslash" => Some("\\"),
        "bracketright" => Some("]"),
        "asciicircum" => Some("^"),
        "underscore" => Some("_"),
        "grave" => Some("`"),
        "braceleft" => Some("{"),
        "bar" => Some("|"),
        "braceright" => Some("}"),
        "asciitilde" => Some("~"),
        "bullet" | "bulletoperator" => Some("•"),
        "endash" | "figuredash" => Some("–"),
        "emdash" => Some("—"),
        "quotedblleft" => Some("“"),
        "quotedblright" => Some("”"),
        "quotedblbase" => Some("„"),
        "quoteleft" => Some("‘"),
        "quoteright" => Some("’"),
        "quotesinglbase" => Some("‚"),
        "guillemotleft" => Some("«"),
        "guillemotright" => Some("»"),
        "guilsinglleft" => Some("‹"),
        "guilsinglright" => Some("›"),
        "ellipsis" => Some("…"),
        "fi" | "f_i" => Some("fi"),
        "fl" | "f_l" => Some("fl"),
        "ff" | "f_f" => Some("ff"),
        "ffi" | "f_f_i" => Some("ffi"),
        "ffl" | "f_f_l" => Some("ffl"),
        "Euro" => Some("€"),
        "sterling" => Some("£"),
        "yen" => Some("¥"),
        "cent" => Some("¢"),
        "section" => Some("§"),
        "paragraph" => Some("¶"),
        "degree" => Some("°"),
        "plusminus" => Some("±"),
        "multiply" => Some("×"),
        "divide" => Some("÷"),
        "periodcentered" | "middot" => Some("·"),
        "copyright" => Some("©"),
        "registered" => Some("®"),
        "trademark" => Some("™"),
        "dagger" => Some("†"),
        "daggerdbl" => Some("‡"),
        "perthousand" => Some("‰"),
        "lessequal" => Some("≤"),
        "greaterequal" => Some("≥"),
        "notequal" => Some("≠"),
        "approxequal" => Some("≈"),
        "infinity" => Some("∞"),
        "partialdiff" => Some("∂"),
        "summation" => Some("∑"),
        "product" => Some("∏"),
        "radical" => Some("√"),
        "integral" => Some("∫"),
        "arrowleft" => Some("←"),
        "arrowright" => Some("→"),
        "arrowup" => Some("↑"),
        "arrowdown" => Some("↓"),
        "checkmark" => Some("✓"),
        "dieresis" => Some("¨"),
        "Aacute" => Some("Á"),
        "aacute" => Some("á"),
        "Agrave" => Some("À"),
        "agrave" => Some("à"),
        "Acircumflex" => Some("Â"),
        "acircumflex" => Some("â"),
        "Adieresis" => Some("Ä"),
        "adieresis" => Some("ä"),
        "Atilde" => Some("Ã"),
        "atilde" => Some("ã"),
        "Aring" => Some("Å"),
        "aring" => Some("å"),
        "AE" => Some("Æ"),
        "ae" => Some("æ"),
        "Ccedilla" => Some("Ç"),
        "ccedilla" => Some("ç"),
        "Eacute" => Some("É"),
        "eacute" => Some("é"),
        "Egrave" => Some("È"),
        "egrave" => Some("è"),
        "Ecircumflex" => Some("Ê"),
        "ecircumflex" => Some("ê"),
        "Edieresis" => Some("Ë"),
        "edieresis" => Some("ë"),
        "Iacute" => Some("Í"),
        "iacute" => Some("í"),
        "Igrave" => Some("Ì"),
        "igrave" => Some("ì"),
        "Icircumflex" => Some("Î"),
        "icircumflex" => Some("î"),
        "Idieresis" => Some("Ï"),
        "idieresis" => Some("ï"),
        "Ntilde" => Some("Ñ"),
        "ntilde" => Some("ñ"),
        "Oacute" => Some("Ó"),
        "oacute" => Some("ó"),
        "Ograve" => Some("Ò"),
        "ograve" => Some("ò"),
        "Ocircumflex" => Some("Ô"),
        "ocircumflex" => Some("ô"),
        "Odieresis" => Some("Ö"),
        "odieresis" => Some("ö"),
        "Otilde" => Some("Õ"),
        "otilde" => Some("õ"),
        "Oslash" => Some("Ø"),
        "oslash" => Some("ø"),
        "OE" => Some("Œ"),
        "oe" => Some("œ"),
        "Uacute" => Some("Ú"),
        "uacute" => Some("ú"),
        "Ugrave" => Some("Ù"),
        "ugrave" => Some("ù"),
        "Ucircumflex" => Some("Û"),
        "ucircumflex" => Some("û"),
        "Udieresis" => Some("Ü"),
        "udieresis" => Some("ü"),
        "Yacute" => Some("Ý"),
        "yacute" => Some("ý"),
        "ydieresis" => Some("ÿ"),
        "Ydieresis" => Some("Ÿ"),
        "germandbls" => Some("ß"),
        "Lslash" => Some("Ł"),
        "lslash" => Some("ł"),
        "Scaron" => Some("Š"),
        "scaron" => Some("š"),
        "Zcaron" => Some("Ž"),
        "zcaron" => Some("ž"),
        "Eth" => Some("Ð"),
        "eth" => Some("ð"),
        "Thorn" => Some("Þ"),
        "thorn" => Some("þ"),
        "ordfeminine" => Some("ª"),
        "ordmasculine" => Some("º"),
        "onesuperior" => Some("¹"),
        "twosuperior" => Some("²"),
        "threesuperior" => Some("³"),
        "onequarter" => Some("¼"),
        "onehalf" => Some("½"),
        "threequarters" => Some("¾"),
        "brokenbar" => Some("¦"),
        "currency" => Some("¤"),
        "florin" => Some("ƒ"),
        "lozenge" => Some("◊"),
        "logicalnot" => Some("¬"),
        "macron" => Some("¯"),
        "prime" | "minute" => Some("′"),
        "second" => Some("″"),
        "equivalence" => Some("≡"),
        "propersubset" => Some("⊂"),
        "propersuperset" => Some("⊃"),
        "reflexsubset" => Some("⊆"),
        "reflexsuperset" => Some("⊇"),
        "notsubset" => Some("⊄"),
        "element" => Some("∈"),
        "notelement" => Some("∉"),
        "suchthat" => Some("∋"),
        "emptyset" => Some("∅"),
        "universal" => Some("∀"),
        "existential" => Some("∃"),
        "gradient" => Some("∇"),
        "angle" => Some("∠"),
        "perpendicular" => Some("⊥"),
        "intersection" => Some("∩"),
        "union" => Some("∪"),
        "proportional" => Some("∝"),
        "similar" => Some("∼"),
        "congruent" => Some("≅"),
        "therefore" => Some("∴"),
        "logicaland" => Some("∧"),
        "logicalor" => Some("∨"),
        "arrowboth" => Some("↔"),
        "carriagereturn" => Some("↵"),
        "arrowdblleft" => Some("⇐"),
        "arrowdblright" => Some("⇒"),
        "arrowdblup" => Some("⇑"),
        "arrowdbldown" => Some("⇓"),
        "arrowdblboth" => Some("⇔"),
        "circlemultiply" => Some("⊗"),
        "circleplus" => Some("⊕"),
        "asteriskmath" => Some("∗"),
        "dotmath" => Some("⋅"),
        "aleph" => Some("ℵ"),
        "Weierstrass" => Some("℘"),
        "Ifraktur" => Some("ℑ"),
        "Rfraktur" => Some("ℜ"),
        "club" => Some("♣"),
        "diamond" => Some("♦"),
        "heart" => Some("♥"),
        "spade" => Some("♠"),
        "H18533" => Some("●"),
        "H18543" => Some("○"),
        "H18551" => Some("▪"),
        "H18552" => Some("▫"),
        "Alpha" => Some("Α"),
        "Beta" => Some("Β"),
        "Gamma" => Some("Γ"),
        "Delta" => Some("Δ"),
        "Epsilon" => Some("Ε"),
        "Zeta" => Some("Ζ"),
        "Eta" => Some("Η"),
        "Theta" => Some("Θ"),
        "Iota" => Some("Ι"),
        "Kappa" => Some("Κ"),
        "Lambda" => Some("Λ"),
        "Mu" => Some("Μ"),
        "Nu" => Some("Ν"),
        "Xi" => Some("Ξ"),
        "Omicron" => Some("Ο"),
        "Pi" => Some("Π"),
        "Rho" => Some("Ρ"),
        "Sigma" => Some("Σ"),
        "Tau" => Some("Τ"),
        "Upsilon" => Some("Υ"),
        "Upsilon1" => Some("ϒ"),
        "Phi" => Some("Φ"),
        "Chi" => Some("Χ"),
        "Psi" => Some("Ψ"),
        "Omega" => Some("Ω"),
        "alpha" => Some("α"),
        "beta" => Some("β"),
        "gamma" => Some("γ"),
        "delta" => Some("δ"),
        "epsilon" => Some("ε"),
        "zeta" => Some("ζ"),
        "eta" => Some("η"),
        "theta" => Some("θ"),
        "theta1" => Some("ϑ"),
        "iota" => Some("ι"),
        "kappa" => Some("κ"),
        "lambda" => Some("λ"),
        "mu" | "mu1" | "micro" => Some("µ"),
        "nu" => Some("ν"),
        "xi" => Some("ξ"),
        "omicron" => Some("ο"),
        "pi" => Some("π"),
        "rho" => Some("ρ"),
        "sigma" => Some("σ"),
        "sigma1" => Some("ς"),
        "tau" => Some("τ"),
        "upsilon" => Some("υ"),
        "phi" => Some("φ"),
        "phi1" => Some("ϕ"),
        "chi" => Some("χ"),
        "psi" => Some("ψ"),
        "omega" => Some("ω"),
        "omega1" => Some("ϖ"),
        "dotlessi" => Some("ı"),
        "acute" => Some("´"),
        "cedilla" => Some("¸"),
        "circumflex" => Some("ˆ"),
        "tilde" => Some("˜"),
        "breve" => Some("˘"),
        "dotaccent" => Some("˙"),
        "ring" => Some("˚"),
        "ogonek" => Some("˛"),
        "hungarumlaut" => Some("˝"),
        "caron" => Some("ˇ"),
        "afii00208" => Some("―"),
        "afii61664" => Some("\u{200C}"),
        // Cyrillic Adobe Glyph List (`afii10017`..`afii10110`)
        "afii10017" => Some("А"),
        "afii10018" => Some("Б"),
        "afii10019" => Some("В"),
        "afii10020" => Some("Г"),
        "afii10021" => Some("Д"),
        "afii10022" => Some("Е"),
        "afii10023" => Some("Ё"),
        "afii10024" => Some("Ж"),
        "afii10025" => Some("З"),
        "afii10026" => Some("И"),
        "afii10027" => Some("Й"),
        "afii10028" => Some("К"),
        "afii10029" => Some("Л"),
        "afii10030" => Some("М"),
        "afii10031" => Some("Н"),
        "afii10032" => Some("О"),
        "afii10033" => Some("П"),
        "afii10034" => Some("Р"),
        "afii10035" => Some("С"),
        "afii10036" => Some("Т"),
        "afii10037" => Some("У"),
        "afii10038" => Some("Ф"),
        "afii10039" => Some("Х"),
        "afii10040" => Some("Ц"),
        "afii10041" => Some("Ч"),
        "afii10042" => Some("Ш"),
        "afii10043" => Some("Щ"),
        "afii10044" => Some("Ъ"),
        "afii10045" => Some("Ы"),
        "afii10046" => Some("Ь"),
        "afii10047" => Some("Э"),
        "afii10048" => Some("Ю"),
        "afii10049" => Some("Я"),
        "afii10065" => Some("а"),
        "afii10066" => Some("б"),
        "afii10067" => Some("в"),
        "afii10068" => Some("г"),
        "afii10069" => Some("д"),
        "afii10070" => Some("е"),
        "afii10071" => Some("ё"),
        "afii10072" => Some("ж"),
        "afii10073" => Some("з"),
        "afii10074" => Some("и"),
        "afii10075" => Some("й"),
        "afii10076" => Some("к"),
        "afii10077" => Some("л"),
        "afii10078" => Some("м"),
        "afii10079" => Some("н"),
        "afii10080" => Some("о"),
        "afii10081" => Some("п"),
        "afii10082" => Some("р"),
        "afii10083" => Some("с"),
        "afii10084" => Some("т"),
        "afii10085" => Some("у"),
        "afii10086" => Some("ф"),
        "afii10087" => Some("х"),
        "afii10088" => Some("ц"),
        "afii10089" => Some("ч"),
        "afii10090" => Some("ш"),
        "afii10091" => Some("щ"),
        "afii10092" => Some("ъ"),
        "afii10093" => Some("ы"),
        "afii10094" => Some("ь"),
        "afii10095" => Some("э"),
        "afii10096" => Some("ю"),
        "afii10097" => Some("я"),
        _ => None,
    }
}
