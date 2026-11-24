use super::*;

fn sample() -> Locale {
    let mut l = Locale::new("en");
    l.add_ron(
        "en",
        r#"{ "play": "Play", "hello": "Hello, {name}!", "only_en": "English only",
            "coins.one": "{n} coin", "coins.other": "{n} coins" }"#,
    )
    .unwrap();
    l.add_ron(
        "ru",
        r#"{ "play": "Играть", "hello": "Привет, {name}!",
            "coins.one": "{n} монета", "coins.few": "{n} монеты", "coins.many": "{n} монет",
            "coins.other": "{n} монеты" }"#,
    )
    .unwrap();
    l
}

#[test]
fn current_language_then_fallback_then_the_key() {
    let mut l = sample();
    assert_eq!(l.tr("play"), "Play");
    assert!(l.set_language("ru"));
    assert_eq!(l.tr("play"), "Играть");
    assert_eq!(l.tr("only_en"), "English only", "falls back to en");
    assert_eq!(l.tr("nope.key"), "nope.key", "the key itself is visible");
}

#[test]
fn regional_and_system_tags_resolve() {
    let mut l = sample();
    assert!(l.set_language("ru_RU.UTF-8"));
    assert_eq!(l.language(), "ru");
    assert!(!l.set_language("de"), "unknown language changes nothing");
    assert_eq!(l.language(), "ru");
    assert_eq!(normalize("C"), "");
    assert_eq!(normalize("en_US.UTF-8"), "en-US");
}

#[test]
fn placeholders_and_literal_braces() {
    let l = sample();
    assert_eq!(l.tr_with("hello", &[("name", &"Dan")]), "Hello, Dan!");
    assert_eq!(fill("{x} and {{x}}", &[("x", &1)]), "1 and {x}");
    assert_eq!(fill("{unknown} stays", &[]), "{unknown} stays");
    assert_eq!(fill("open { brace", &[]), "open { brace");
}

#[test]
fn plurals_follow_the_language() {
    let mut l = sample();
    assert_eq!(l.plural("coins", 1, &[]), "1 coin");
    assert_eq!(l.plural("coins", 5, &[]), "5 coins");
    l.set_language("ru");
    for (n, word) in [
        (1, "монета"),
        (2, "монеты"),
        (5, "монет"),
        (11, "монет"),
        (21, "монета"),
        (22, "монеты"),
        (112, "монет"),
    ] {
        assert_eq!(l.plural("coins", n, &[]), format!("{n} {word}"));
    }
}

#[test]
fn plural_rules_of_other_languages() {
    assert_eq!(plural_form("ja", 5), "other");
    assert_eq!(plural_form("fr", 0), "one");
    assert_eq!(plural_form("pl", 22), "few");
    assert_eq!(plural_form("pl", 5), "many");
    assert_eq!(plural_form("cs", 3), "few");
    assert_eq!(plural_form("en", -1), "one");
}

#[test]
fn translators_get_the_missing_keys() {
    let l = sample();
    assert_eq!(l.missing_in("ru"), vec!["only_en"]);
    assert!(l.missing_in("en").is_empty());
}

#[test]
fn a_directory_loads_and_a_broken_file_is_skipped() {
    let dir = std::env::temp_dir().join(format!("olvin-locale-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("en.ron"), r#"{ "a": "A" }"#).unwrap();
    std::fs::write(dir.join("de.ron"), "{ not ron").unwrap();
    std::fs::write(dir.join("readme.txt"), "x").unwrap();
    let mut l = Locale::new("en");
    assert_eq!(l.load_dir(&dir), 1);
    assert_eq!(l.languages(), vec!["en"]);
    assert_eq!(l.load_dir(&dir.join("missing")), 0);
}
