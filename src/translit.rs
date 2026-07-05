use wana_kana::{ConvertJapanese, Options};

/// Hiragana codepoint block (0x3040–0x309F).
fn has_hiragana(s: &str) -> bool {
    s.chars().any(|c| (0x3040..=0x309F).contains(&(c as u32)))
}

/// Katakana codepoint blocks: full-width (0x30A0–0x30FF) and half-width (0xFF66–0xFF9F).
fn has_katakana(s: &str) -> bool {
    s.chars().any(|c| {
        let v = c as u32;
        (0x30A0..=0x30FF).contains(&v) || (0xFF66..=0xFF9F).contains(&v)
    })
}

/// Normalise a romaji string for fuzzy cross-script search.
///
/// `wana_kana` expands the katakana prolonged-sound mark `ー` (chōonpu) into a
/// repeated vowel when romanising katakana (e.g. `ブルーバード` → `buruubaado`),
/// and into an ASCII hyphen when romanising hiragana (e.g. `ぶるーばーど` →
/// `buru-ba-do`). Neither form matches what a user actually types when searching
/// for the title — they type `burubado`. To preserve that intent we:
///   1. drop hyphens (the hiragana-`ー` rendering), and
///   2. collapse runs of the same vowel into a single character (the
///      katakana-`ー` rendering, e.g. `uu` → `u`, `aa` → `a`).
///
/// Consonants and non-vowel characters are left untouched, so legitimate
/// doubled consonants (e.g. `kappa`) are preserved.
fn normalize_romaji(s: &str) -> String {
    let no_hyphen: String = s.chars().filter(|&c| c != '-').collect();
    let mut out = String::with_capacity(no_hyphen.len());
    let mut prev: Option<char> = None;
    for c in no_hyphen.chars() {
        let is_vowel = matches!(c, 'a' | 'i' | 'u' | 'e' | 'o' | 'A' | 'I' | 'U' | 'E' | 'O');
        if is_vowel && prev == Some(c) {
            // Collapse repeated vowel produced by an expanded chōonpu.
            continue;
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

/// Return alternate-script variants of `text` for cross-script search.
///
/// - katakana text → romaji + hiragana
/// - hiragana text → romaji + katakana
///
/// ASCII-only or kanji-only text yields no variants (kanji→romaji needs a
/// dictionary, which we deliberately do not ship).
///
/// The original text is NOT included here; the caller indexes it separately.
///
/// Note on the romaji variant: `wana_kana` renders the katakana long-vowel mark
/// `ー` either as a doubled vowel (katakana input) or as `-` (hiragana input).
/// We post-process the romaji via [`normalize_romaji`] so that searching for
/// `burubado` matches the title `ブルーバード`.
pub fn variants(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    // `IsJapaneseStr::is_kana` returns true only when the WHOLE string is kana,
    // so we detect "contains kana" via codepoint ranges instead.
    let has_kana = has_katakana(text) || has_hiragana(text);
    if !has_kana {
        return out;
    }

    // to_romaji converts kana→romaji and passes through non-kana chars (ascii,
    // kanji) unchanged. We then normalise the chōonpu expansions so the romaji
    // form matches what a user types.
    let romaji = normalize_romaji(&text.to_romaji());
    if romaji != text {
        out.push(romaji);
    }

    // Preserve the `ー` mark when converting katakana→hiragana so that
    // `ブルーバード` → `ぶるーばーど` (rather than the default `ぶるうばあど`).
    // `pass_romaji` keeps embedded ascii (e.g. "Ado") intact in mixed input.
    let hiragana_opts = Options {
        keep_prolonged_sound_mark: true,
        pass_romaji: true,
        ..Default::default()
    };
    if has_katakana(text) {
        let h = text.to_hiragana_with_opt(hiragana_opts);
        if h != text {
            out.push(h);
        }
    }
    if has_hiragana(text) {
        let k = text.to_katakana();
        if k != text {
            out.push(k);
        }
    }

    out.sort();
    out.dedup();
    out
}
