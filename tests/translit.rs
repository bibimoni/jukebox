use jukebox::translit::variants;

#[test]
fn katakana_yields_romaji_and_hiragana() {
    let v = variants("ブルーバード");
    assert!(v.iter().any(|s| s == "burubado"), "got romaji: {:?}", v);
    assert!(
        v.iter().any(|s| s == "ぶるーばーど"),
        "got hiragana: {:?}",
        v
    );
}

#[test]
fn hiragana_yields_romaji_and_katakana() {
    let v = variants("ぶるーばーど");
    assert!(v.iter().any(|s| s == "burubado"));
    assert!(v.iter().any(|s| s == "ブルーバード"));
}

#[test]
fn ascii_only_yields_no_variants() {
    let v = variants("Blue Bird");
    assert!(v.is_empty(), "got: {:?}", v);
}

#[test]
fn variants_are_deduped() {
    let v = variants("カナカナ");
    assert_eq!(v.len(), 2); // romaji + hiragana
}

#[test]
fn mixed_kana_ascii_still_transliterates_kana() {
    let v = variants("Ado ブルーバード");
    assert!(v.iter().any(|s| s.contains("burubado")));
}
