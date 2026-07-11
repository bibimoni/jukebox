//! Local-first matching for Mixed mode (spec §4.1).
//!
//! Given a YouTube [`RemoteTrack`], find the local catalog track it most
//! likely *is* — so Mixed mode plays the hi-res local copy instead of
//! streaming the lossy YouTube version. Conservative: a wrong local
//! substitution (playing the wrong song) is worse than streaming, so the
//! thresholds are high and the borderline band [0.80, 0.88) is rejected.

use crate::catalog::Catalog;
use crate::source::RemoteTrack;
use crate::translit::variants;

/// Title similarity required to promote a YouTube track to its local copy.
const TITLE_GATE: f64 = 0.88;
/// Below this artist similarity, reject outright regardless of title.
const ARTIST_FLOOR: f64 = 0.80;

/// Find the local catalog track matching `remote`, or `None`.
pub fn match_local(remote: &RemoteTrack, cat: &Catalog) -> Option<String> {
    // 1. ISRC (strong). Case-insensitive exact match — deterministic.
    if let Some(isrc) = remote.isrc.as_deref().filter(|s| !s.is_empty()) {
        let want = isrc.to_ascii_lowercase();
        for t in &cat.tracks {
            if let Some(have) = t.isrc.as_deref() {
                if have.to_ascii_lowercase() == want {
                    return Some(t.id.clone());
                }
            }
        }
    }

    // 2. Normalized artist+title fuzzy. Compare title and artist independently
    // (a combined ratio is inflated by a long matching artist).
    let r_title = norm(&remote.title);
    let r_artist = norm(&remote.artist);
    if r_title.is_empty() || r_artist.is_empty() {
        return None;
    }
    // Variants let kana/romaji cross-match (existing translit module).
    let r_title_variants: Vec<String> = variants(&remote.title)
        .into_iter()
        .map(|v| norm(&v))
        .chain(std::iter::once(r_title.clone()))
        .collect();
    let r_artist_variants: Vec<String> = variants(&remote.artist)
        .into_iter()
        .map(|v| norm(&v))
        .chain(std::iter::once(r_artist.clone()))
        .collect();

    let mut best: Option<(f64, String)> = None;
    for t in &cat.tracks {
        let c_title = norm(&t.title);
        let c_artist = norm(&t.primary_artist);
        if c_title.is_empty() || c_artist.is_empty() {
            continue;
        }
        // Artist must clear its floor on at least one variant pair (or match
        // exactly), else reject regardless of title.
        let artist_ok = r_artist == c_artist
            || r_artist_variants
                .iter()
                .any(|ra| ratio(ra, &c_artist) >= ARTIST_FLOOR);
        if !artist_ok {
            continue;
        }
        // Title ratio: best over remote/candidate variant pairs.
        let c_title_variants = variants(&t.title);
        let mut title_ratio = 0.0_f64;
        for rt in &r_title_variants {
            let mut best_for_rt = ratio(rt, &c_title);
            for cv in &c_title_variants {
                best_for_rt = best_for_rt.max(ratio(rt, &norm(cv)));
            }
            title_ratio = title_ratio.max(best_for_rt);
        }
        if title_ratio >= TITLE_GATE {
            best = match best {
                Some((b, _)) if title_ratio <= b => best,
                _ => Some((title_ratio, t.id.clone())),
            };
        }
    }
    best.map(|(_, id)| id)
}

/// Normalize for fuzzy compare: lowercase, drop `feat.*`/`ft.*` clauses, keep
/// alphanumerics and CJK, collapse whitespace.
fn norm(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut skip_feat = false;
    for w in lower.split_whitespace() {
        if skip_feat {
            continue;
        }
        if matches!(w, "feat." | "feat" | "ft." | "ft") {
            skip_feat = true;
            continue;
        }
        for c in w.chars() {
            // keep alphanumerics and CJK/punct ranges (>= 0x3000)
            if c.is_alphanumeric() || (c as u32) >= 0x3000 {
                out.push(c);
            }
        }
        out.push(' ');
    }
    out.trim_end().to_string()
}

/// Normalized Levenshtein similarity ratio in [0,1]: 1 - edit/longer_len.
fn ratio(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let dist = levenshtein(&a, &b) as f64;
    let longer = a.len().max(b.len()).max(1) as f64;
    1.0 - dist / longer
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let (n, m) = (a.len(), b.len());
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        cur[0] = i;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_strips_feat_and_punct() {
        assert_eq!(norm("Dawn feat. Someone"), "dawn");
        assert_eq!(norm("Hello! (Live)"), "hello live");
    }

    #[test]
    fn ratio_identical_is_one() {
        assert_eq!(ratio("adele hello", "adele hello"), 1.0);
    }
}
