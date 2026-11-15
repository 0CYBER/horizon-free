use crate::scan::{Token, tokens};

/// Every identifier of `text` with its 1-based line, in order of appearance.
fn identifiers(text: &str) -> impl Iterator<Item = (&str, usize)> {
    tokens(text).filter_map(|(token, line)| match token {
        Token::Ident(name) => Some((name, line)),
        _ => None,
    })
}

fn names(text: &str) -> Vec<&str> {
    identifiers(text).map(|(name, _)| name).collect()
}

#[test]
fn identifiers_in_code_count_with_their_lines() {
    let text = "use mod_api::Hooks;\n\nfn tick(host: &dyn Host) {\n    host.give_item();\n}\n";
    let found: Vec<(&str, usize)> = identifiers(text).collect();
    assert_eq!(
        found,
        [
            ("use", 1),
            ("mod_api", 1),
            ("Hooks", 1),
            ("fn", 3),
            ("tick", 3),
            ("host", 3),
            ("dyn", 3),
            ("Host", 3),
            ("host", 4),
            ("give_item", 4),
        ]
    );
}

#[test]
fn comments_are_skipped_nested_block_comments_included() {
    let text = "a // line b\n/* c /* d */ e */ f /** g */ h /// i\nj";
    assert_eq!(names(text), ["a", "f", "h", "j"]);
    // Lines still count inside a block comment.
    let found: Vec<(&str, usize)> = identifiers("/* x\n\n*/ after").collect();
    assert_eq!(found, [("after", 3)]);
}

#[test]
fn string_and_char_literals_are_skipped() {
    let text = r#"a "b \" c" d b"e" f c"g" h '"' i '\'' j '\u{41}' k 'é' l b'm' n"#;
    assert_eq!(names(text), ["a", "d", "f", "h", "i", "j", "k", "l", "n"]);
}

#[test]
fn raw_strings_end_at_their_own_hashes() {
    let text = "a r\"b\" c r#\"d \"e\" f\"# g br##\"h \"# i\"## j cr\"k\" l";
    assert_eq!(names(text), ["a", "c", "g", "j", "l"]);
}

#[test]
fn lifetimes_and_labels_are_not_names_and_do_not_open_a_literal() {
    let text = "fn f<'a, 'host>(x: &'a Text) { 'outer: loop { break 'outer; } give }";
    assert_eq!(
        names(text),
        ["fn", "f", "x", "Text", "loop", "break", "give"]
    );
}

#[test]
fn raw_identifiers_count_by_their_name_and_numbers_do_not() {
    assert_eq!(
        names("r#type 1u32 0x_ffu8 1e10 2.5f32 0..3 r#match"),
        ["type", "match"]
    );
}

#[test]
fn macro_bodies_are_scanned() {
    assert_eq!(
        names("vec![ItemStack::EMPTY; 3]"),
        ["vec", "ItemStack", "EMPTY"]
    );
}

#[test]
fn unterminated_input_ends_the_scan_without_a_panic() {
    let cases: [(&str, &[&str]); 9] = [
        ("a \"open b", &["a"]),
        ("a /* open b", &["a"]),
        ("a r#\"open b", &["a"]),
        ("a '", &["a"]),
        ("a '\\", &["a"]),
        ("a b'", &["a"]),
        ("a r#", &["a"]),
        ("a r", &["a", "r"]),
        ("é'x", &["é"]),
    ];
    for (text, expected) in cases {
        assert_eq!(names(text), expected, "{text}");
    }
}

#[test]
fn tokens_keep_punctuation_literals_and_lifetimes_in_order() {
    let found: Vec<Token<'_>> = tokens("S { a: 'x', ..f::<'b>(\"s\", 1) } // c")
        .map(|(token, _)| token)
        .collect();
    assert_eq!(
        found,
        [
            Token::Ident("S"),
            Token::Punct(b'{'),
            Token::Ident("a"),
            Token::Punct(b':'),
            Token::Literal("'x'"),
            Token::Punct(b','),
            Token::Punct(b'.'),
            Token::Punct(b'.'),
            Token::Ident("f"),
            Token::Punct(b':'),
            Token::Punct(b':'),
            Token::Punct(b'<'),
            Token::Lifetime,
            Token::Punct(b'>'),
            Token::Punct(b'('),
            Token::Literal("\"s\""),
            Token::Punct(b','),
            Token::Literal("1"),
            Token::Punct(b')'),
            Token::Punct(b'}'),
        ]
    );
}

#[test]
fn a_tuple_fields_position_after_a_single_dot_is_a_name() {
    assert_eq!(names("id.0 > 7"), ["id", "0"]);
    assert_eq!(names("pair.0.1"), ["pair", "0", "1"]);
    // A float, a range and a method on a number are not.
    assert_eq!(names("let x = 1.5 + 0..2;"), ["let", "x"]);
    let found: Vec<(Token<'_>, usize)> = crate::scan::tokens("1.25").collect();
    assert_eq!(found, [(Token::Literal("1.25"), 1)]);
}

#[test]
fn a_tuple_position_after_a_dot_and_spaces_or_a_comment_is_a_name() {
    let names: Vec<&str> = identifiers("p. 0 + p./**/1 + p.\n2 + 0..3 + 1.5")
        .map(|(n, _)| n)
        .collect();
    assert_eq!(names, ["p", "0", "p", "1", "p", "2"]);
}
