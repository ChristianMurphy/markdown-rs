use markdown::{
    extension::{Construct, ConstructTokenizer, Step, Token},
    mdast::{Custom, Node, Paragraph, Root, Text},
    to_html_with_options, to_mdast,
    unist::Position,
    Options, ParseOptions,
};
use pretty_assertions::assert_eq;

/// `@` followed by one byte that is not whitespace, turned into a `mention`
/// node that keeps the token values it was given.
struct Mention {
    marker: u8,
    /// Second bytes this construct accepts.
    accept: &'static [u8],
}

impl Construct for Mention {
    fn markers(&self) -> &[u8] {
        core::slice::from_ref(&self.marker)
    }

    fn step(&self, state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current()) {
            (0, Some(byte)) if byte == self.marker => {
                tokenizer.enter("mention");
                tokenizer.enter("mentionMarker");
                tokenizer.consume();
                tokenizer.exit("mentionMarker");
                Step::Next(1)
            }
            (1, Some(byte)) if self.accept.contains(&byte) => {
                tokenizer.enter("mentionName");
                tokenizer.consume();
                tokenizer.exit("mentionName");
                tokenizer.exit("mention");
                Step::Ok
            }
            _ => Step::Nok,
        }
    }

    fn to_mdast(&self, tokens: Vec<Token>) -> Node {
        Node::Custom(Custom {
            name: "mention".into(),
            attributes: tokens
                .iter()
                .map(|token| (token.name.into(), token.value.clone().into_owned()))
                .collect(),
            ..Custom::default()
        })
    }
}

fn options(constructs: Vec<Mention>) -> ParseOptions {
    ParseOptions {
        text_constructs: constructs
            .into_iter()
            .map(|construct| Box::new(construct) as Box<dyn Construct>)
            .collect(),
        ..ParseOptions::default()
    }
}

fn mention() -> Mention {
    Mention {
        marker: b'@',
        accept: b"abc<",
    }
}

/// Children of the first paragraph.
fn phrasing(tree: Node) -> Vec<Node> {
    match tree {
        Node::Root(Root { mut children, .. }) => match children.remove(0) {
            Node::Paragraph(Paragraph { children, .. }) => children,
            node => panic!("expected paragraph, got {:?}", node),
        },
        node => panic!("expected root, got {:?}", node),
    }
}

fn custom(node: &Node) -> &Custom {
    match node {
        Node::Custom(custom) => custom,
        node => panic!("expected custom node, got {:?}", node),
    }
}

#[test]
fn construct_makes_node_from_its_tokens() {
    let children = phrasing(to_mdast("x @a y", &options(vec![mention()])).unwrap());
    let node = custom(&children[1]);

    assert_eq!(
        node.attributes
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("mention", "@a"),
            ("mentionMarker", "@"),
            ("mentionName", "a")
        ],
        "should pass each token's name and value"
    );
    assert_eq!(
        node.position,
        Some(Position::new(1, 3, 2, 1, 5, 4)),
        "should give the node the position of the outer token"
    );
    assert_eq!(children.len(), 3, "should keep the text around it");
}

#[test]
fn falls_back_to_text_when_construct_does_not_match() {
    assert_eq!(
        phrasing(to_mdast("x @z y", &options(vec![mention()])).unwrap()),
        vec![Node::Text(Text {
            value: "x @z y".into(),
            position: Some(Position::new(1, 1, 0, 1, 7, 6)),
        })],
        "should discard the construct's events and keep the bytes as text"
    );
}

#[test]
fn works_inside_containers() {
    let tree = to_mdast("> x\n> @a", &options(vec![mention()])).unwrap();
    let quote = tree.children().unwrap()[0].children().unwrap();
    let children = quote[0].children().unwrap();

    let node = children
        .iter()
        .find(|node| matches!(node, Node::Custom(_)))
        .expect("expected a mention");

    assert_eq!(
        custom(node).attributes.get("mention").map(String::as_str),
        Some("@a"),
        "should see the stream without the `> ` prefix"
    );
}

#[test]
fn runs_before_builtin_constructs() {
    let star = Mention {
        marker: b'*',
        accept: b"a",
    };
    let children = phrasing(to_mdast("*a*", &options(vec![star])).unwrap());

    assert!(
        matches!(children[0], Node::Custom(_)),
        "should try the construct before emphasis at `*`"
    );
}

#[test]
fn tries_next_construct_after_no_match() {
    let only_a = Mention {
        marker: b'@',
        accept: b"a",
    };
    let only_b = Mention {
        marker: b'@',
        accept: b"b",
    };
    let children = phrasing(to_mdast("@a @b", &options(vec![only_a, only_b])).unwrap());

    assert_eq!(
        custom(&children[2])
            .attributes
            .get("mentionName")
            .map(String::as_str),
        Some("b"),
        "should try the second construct when the first does not match"
    );
}

#[test]
fn to_html_writes_construct_source_as_text() {
    let options = Options {
        parse: options(vec![mention()]),
        ..Options::default()
    };

    assert_eq!(
        to_html_with_options("x @<http://a> y", &options).unwrap(),
        "<p>x @&lt;http://a&gt; y</p>",
        "should keep and encode the source of constructs it cannot render"
    );
}

/// A step of a scripted construct.
type StepFn = fn(u16, &mut ConstructTokenizer) -> Step;

/// A construct scripted by a function, with `tokens[0].value` as its value.
struct Scripted {
    marker: u8,
    step: StepFn,
}

impl Construct for Scripted {
    fn markers(&self) -> &[u8] {
        core::slice::from_ref(&self.marker)
    }

    fn step(&self, state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        (self.step)(state, tokenizer)
    }

    fn to_mdast(&self, tokens: Vec<Token>) -> Node {
        Node::Custom(Custom {
            name: "scripted".into(),
            value: Some(tokens[0].value.clone().into_owned()),
            fields: vec![(
                "tokens".into(),
                tokens
                    .iter()
                    .map(|token| token.name)
                    .collect::<Vec<_>>()
                    .join(","),
            )]
            .into_iter()
            .collect(),
            attributes: tokens[1..]
                .iter()
                .map(|token| (token.name.into(), token.value.clone().into_owned()))
                .collect(),
            ..Custom::default()
        })
    }
}

/// `{{`, anything including line endings (as `bracesData`), `}}`.
fn braces(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (0, Some(b'{')) => {
            tokenizer.enter("braces");
            tokenizer.consume();
            Step::Next(1)
        }
        (1, Some(b'{')) => {
            tokenizer.consume();
            tokenizer.enter("bracesData");
            Step::Next(2)
        }
        (2, Some(b'}')) => {
            tokenizer.exit("bracesData");
            tokenizer.consume();
            Step::Next(3)
        }
        (2, Some(_)) => {
            tokenizer.consume();
            Step::Next(2)
        }
        (3, Some(b'}')) => {
            tokenizer.consume();
            tokenizer.exit("braces");
            Step::Ok
        }
        _ => Step::Nok,
    }
}

fn scripted(step: StepFn) -> ParseOptions {
    ParseOptions {
        text_constructs: vec![Box::new(Scripted { marker: b'{', step })],
        ..ParseOptions::default()
    }
}

fn find_scripted(node: &Node) -> Option<&Custom> {
    match node {
        Node::Custom(custom) if custom.name == "scripted" => Some(custom),
        _ => node.children()?.iter().find_map(find_scripted),
    }
}

#[test]
fn joins_tokens_across_lines_without_container_prefixes() {
    for input in ["> {{a\n> b}}", "- {{a\n  b}}", "{{a\nb}}"] {
        let tree = to_mdast(input, &scripted(braces)).unwrap();
        let node = find_scripted(&tree).expect("expected a match");

        assert_eq!(
            node.value.as_deref(),
            Some("{{a\nb}}"),
            "should join the outer token without prefixes in {:?}",
            input
        );
        assert_eq!(
            node.attributes.get("bracesData").map(String::as_str),
            Some("a\nb"),
            "should join inner tokens without prefixes in {:?}",
            input
        );
    }
}

#[test]
fn to_html_writes_multiline_source_without_prefixes() {
    let options = Options {
        parse: scripted(braces),
        ..Options::default()
    };

    assert_eq!(
        to_html_with_options("> {{*a*\n> b}}", &options).unwrap(),
        "<blockquote>\n<p>{{*a*\nb}}</p>\n</blockquote>"
    );
}

#[test]
fn broken_constructs_leave_text() {
    let cases: Vec<(&str, StepFn)> = vec![
        ("`Ok` without consuming", |_, _| Step::Ok),
        ("`Next` without consuming", |_, tokenizer| {
            tokenizer.enter("a");
            Step::Next(0)
        }),
        ("`Retry` forever", |_, _| Step::Retry(0)),
        ("consuming outside a token", |_, tokenizer| {
            tokenizer.consume();
            Step::Ok
        }),
        (
            "consuming after the outermost token",
            |state, tokenizer| match state {
                0 => {
                    tokenizer.enter("a");
                    tokenizer.consume();
                    tokenizer.exit("a");
                    Step::Next(1)
                }
                1 => {
                    tokenizer.consume();
                    Step::Next(2)
                }
                _ => Step::Ok,
            },
        ),
        ("`Retry` after consuming", |state, tokenizer| match state {
            0 => {
                tokenizer.enter("a");
                tokenizer.consume();
                Step::Retry(1)
            }
            _ => {
                tokenizer.consume();
                tokenizer.exit("a");
                Step::Ok
            }
        }),
        ("closing a token that is not open", |_, tokenizer| {
            tokenizer.enter("a");
            tokenizer.consume();
            tokenizer.exit("b");
            Step::Ok
        }),
        ("`Ok` with an open token", |_, tokenizer| {
            tokenizer.enter("a");
            tokenizer.consume();
            Step::Ok
        }),
        ("`Ok` with the outermost token open", |_, tokenizer| {
            tokenizer.enter("a");
            tokenizer.enter("b");
            tokenizer.consume();
            tokenizer.exit("b");
            Step::Ok
        }),
        ("two outermost tokens", |state, tokenizer| match state {
            0 => {
                tokenizer.enter("a");
                tokenizer.consume();
                tokenizer.exit("a");
                tokenizer.enter("b");
                Step::Next(1)
            }
            _ => {
                tokenizer.consume();
                tokenizer.exit("b");
                Step::Ok
            }
        }),
        ("an empty token", |_, tokenizer| {
            tokenizer.enter("a");
            tokenizer.exit("a");
            tokenizer.enter("b");
            tokenizer.consume();
            tokenizer.exit("b");
            Step::Ok
        }),
    ];

    for (label, step) in cases {
        let tree = to_mdast("x {y", &scripted(step)).unwrap();
        assert!(
            find_scripted(&tree).is_none(),
            "should not match a construct that breaks a rule: {}",
            label
        );
        assert_eq!(
            tree.to_string(),
            "x {y",
            "should keep text for a construct that breaks a rule: {}",
            label
        );
    }
}

#[test]
fn tokens_inside_a_character_leave_text() {
    let cases: Vec<(&str, u8, StepFn)> = vec![
        (
            "an outer token ending",
            b'{',
            |state, tokenizer| match state {
                0 => {
                    tokenizer.enter("a");
                    tokenizer.consume();
                    Step::Next(1)
                }
                _ => {
                    // Consume one byte of the two in `é`.
                    tokenizer.consume();
                    tokenizer.exit("a");
                    Step::Ok
                }
            },
        ),
        (
            "an inner token ending",
            b'{',
            |state, tokenizer| match state {
                0 => {
                    tokenizer.enter("a");
                    tokenizer.consume();
                    Step::Next(1)
                }
                1 => {
                    tokenizer.enter("b");
                    tokenizer.consume();
                    tokenizer.exit("b");
                    Step::Next(2)
                }
                _ => {
                    tokenizer.consume();
                    tokenizer.exit("a");
                    Step::Ok
                }
            },
        ),
        // The second byte of `é`.
        ("a token starting", 0xA9, |_, tokenizer| {
            tokenizer.enter("a");
            tokenizer.consume();
            tokenizer.exit("a");
            Step::Ok
        }),
    ];

    for (label, marker, step) in cases {
        let parse = ParseOptions {
            text_constructs: vec![Box::new(Scripted { marker, step })],
            ..ParseOptions::default()
        };
        let options = Options {
            parse,
            ..Options::default()
        };

        assert_eq!(
            to_html_with_options("{é", &options).unwrap(),
            "<p>{é</p>",
            "should keep text for {}",
            label
        );
    }
}

#[test]
fn resets_retries_after_each_byte() {
    /// `{`, then `a`s, each after a `Retry`.
    fn retry_each(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current()) {
            (0, Some(b'{')) => {
                tokenizer.enter("a");
                tokenizer.consume();
                Step::Next(1)
            }
            (1, Some(b'a')) => Step::Retry(2),
            (2, Some(b'a')) => {
                tokenizer.consume();
                Step::Next(1)
            }
            (1, _) => {
                tokenizer.exit("a");
                Step::Ok
            }
            _ => Step::Nok,
        }
    }

    let input = format!("{{{}", "a".repeat(300));
    let mut found = vec![];
    values(
        &to_mdast(&input, &scripted(retry_each)).unwrap(),
        &mut found,
    );

    assert_eq!(
        found,
        vec![input],
        "should allow a `Retry` before each byte of a long match"
    );
}

#[test]
fn line_endings_are_never_markers() {
    let parse = ParseOptions {
        text_constructs: vec![Box::new(Scripted {
            marker: b'\n',
            step: |_, _| unreachable!("should not start at a line ending"),
        })],
        ..ParseOptions::default()
    };
    let options = Options {
        parse,
        ..Options::default()
    };

    assert_eq!(
        to_html_with_options("a  \nb", &options).unwrap(),
        "<p>a<br />\nb</p>",
        "should keep hard breaks working"
    );
}

/// `%`, then bytes, as one token, ending before a line ending or at the end.
fn percent_to_line_end(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (0, Some(b'%')) => {
            tokenizer.enter("percent");
            tokenizer.consume();
            Step::Next(1)
        }
        (1, Some(b'\n') | None) => {
            tokenizer.exit("percent");
            Step::Ok
        }
        (1, Some(_)) => {
            tokenizer.consume();
            Step::Next(1)
        }
        _ => Step::Nok,
    }
}

/// Like `percent_to_line_end`, but wrongly consumes the line ending before
/// ending.
fn percent_through_line_end(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (1, Some(b'\n')) => {
            tokenizer.consume();
            tokenizer.exit("percent");
            Step::Ok
        }
        _ => percent_to_line_end(state, tokenizer),
    }
}

fn percent(step: StepFn) -> ParseOptions {
    ParseOptions {
        text_constructs: vec![Box::new(Scripted { marker: b'%', step })],
        ..ParseOptions::default()
    }
}

fn values(node: &Node, values: &mut Vec<String>) {
    match node {
        Node::Custom(custom) => values.push(custom.value.clone().unwrap_or_default()),
        _ => {
            for child in node.children().into_iter().flatten() {
                self::values(child, values);
            }
        }
    }
}

#[test]
fn ends_before_a_line_ending_without_consuming_it() {
    let mut found = vec![];
    values(
        &to_mdast("> %a\n> %b", &percent(percent_to_line_end)).unwrap(),
        &mut found,
    );

    assert_eq!(found, vec!["%a", "%b"], "should end at the line ending");
}

#[test]
fn does_not_end_right_after_consuming_a_line_ending() {
    let mut found = vec![];
    values(
        &to_mdast("> %a\n> %b", &percent(percent_through_line_end)).unwrap(),
        &mut found,
    );

    assert_eq!(
        found,
        vec!["%b"],
        "should reject the first match, and keep the later one"
    );
}

#[test]
fn does_not_start_a_token_right_after_consuming_a_line_ending() {
    /// `braces`, with a `line` token from each line ending to the next `}`.
    fn line_after_line_ending(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current()) {
            (2, Some(b'\n')) => {
                tokenizer.consume();
                tokenizer.enter("line");
                Step::Next(5)
            }
            (5, Some(b'}')) => {
                tokenizer.exit("line");
                Step::Retry(2)
            }
            (5, Some(_)) => {
                tokenizer.consume();
                Step::Next(5)
            }
            _ => braces(state, tokenizer),
        }
    }

    let tree = to_mdast("> {{a\n> b}}", &scripted(line_after_line_ending)).unwrap();
    assert!(
        find_scripted(&tree).is_none(),
        "should reject a token started before the next prefix is skipped"
    );
}

#[test]
fn ends_a_token_after_a_line_ending() {
    fn line_ending_then_ok(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current()) {
            (1, Some(b'\n')) => {
                tokenizer.consume();
                tokenizer.exit("percent");
                Step::Next(2)
            }
            (2, _) => Step::Ok,
            _ => percent_to_line_end(state, tokenizer),
        }
    }

    let mut found = vec![];
    values(
        &to_mdast("> %a\n> %b", &percent(line_ending_then_ok)).unwrap(),
        &mut found,
    );

    assert_eq!(
        found,
        vec!["%a\n", "%b"],
        "should keep the line ending in the token, without the next prefix"
    );
}

#[test]
fn keeps_line_endings_in_tokens() {
    for (input, expected) in [
        ("> {{\n> }}", "\n"),
        ("> {{\n> a}}", "\na"),
        ("> {{\r\n> a}}", "\r\na"),
    ] {
        let tree = to_mdast(input, &scripted(braces)).unwrap();
        let node = find_scripted(&tree).expect("expected a match");

        assert_eq!(
            node.attributes.get("bracesData").map(String::as_str),
            Some(expected),
            "should keep the line ending, without the next prefix, in {:?}",
            input
        );
    }
}

#[test]
fn leaves_out_tabs_used_by_prefixes() {
    let tree = to_mdast("-\t{{a\n\tb}}", &scripted(braces)).unwrap();

    assert_eq!(
        find_scripted(&tree).and_then(|node| node.value.as_deref()),
        Some("{{a\nb}}"),
        "should match `to_html` and code (text), which leave the tab out"
    );
}

#[test]
fn consuming_at_the_end_does_nothing() {
    fn to_end(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current()) {
            (0, _) => {
                tokenizer.enter("a");
                tokenizer.consume();
                Step::Next(1)
            }
            (_, Some(_)) => {
                tokenizer.consume();
                Step::Next(1)
            }
            (_, None) => {
                tokenizer.consume();
                tokenizer.exit("a");
                Step::Ok
            }
        }
    }

    let tree = to_mdast("x {y", &scripted(to_end)).unwrap();
    assert_eq!(
        find_scripted(&tree).and_then(|node| node.value.as_deref()),
        Some("{y"),
        "should match up to the end, without panicking"
    );
}

/// `mention`, after bytes that `allow` accepts.
struct After(fn(Option<u8>) -> bool);

impl Construct for After {
    fn markers(&self) -> &[u8] {
        b"@"
    }

    fn previous(&self, previous: Option<u8>) -> bool {
        (self.0)(previous)
    }

    fn step(&self, state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        mention().step(state, tokenizer)
    }

    fn to_mdast(&self, tokens: Vec<Token>) -> Node {
        mention().to_mdast(tokens)
    }
}

fn has_custom(node: &Node) -> bool {
    matches!(node, Node::Custom(_))
        || node
            .children()
            .map_or(false, |children| children.iter().any(has_custom))
}

#[test]
fn checks_the_byte_before() {
    let not_after_star = ParseOptions {
        text_constructs: vec![Box::new(After(|previous| previous != Some(b'*')))],
        ..ParseOptions::default()
    };

    for (input, expected) in [
        ("@a", true),
        ("x @a", true),
        ("*@a", false),
        ("\\*@a", true),
    ] {
        assert_eq!(
            has_custom(&to_mdast(input, &not_after_star).unwrap()),
            expected,
            "should start only after allowed bytes, with `None` after an escape, in {:?}",
            input
        );
    }

    let at_line_start = ParseOptions {
        text_constructs: vec![Box::new(After(|previous| {
            matches!(previous, None | Some(b'\n'))
        }))],
        ..ParseOptions::default()
    };

    for (input, expected) in [
        ("x\n@a", true),
        ("> x\n> @a", true),
        ("> x\n>@a", true),
        ("> x\n@a", true),
        ("- x\n  @a", true),
        ("x\r\n@a", true),
        ("x @a", false),
    ] {
        assert_eq!(
            has_custom(&to_mdast(input, &at_line_start).unwrap()),
            expected,
            "should see a line ending before a line, without container prefixes, in {:?}",
            input
        );
    }
}

#[test]
fn keeps_memory_for_one_match() {
    /// A run of `{`, other bytes, and a run of `}` of the same size.
    fn balanced(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current()) {
            (0, Some(b'{')) => {
                tokenizer.enter("balanced");
                tokenizer.consume();
                tokenizer.memory()[0] = 1;
                Step::Next(1)
            }
            (1, Some(b'{')) => {
                tokenizer.consume();
                tokenizer.memory()[0] += 1;
                Step::Next(1)
            }
            (1..=3, Some(b'}')) => {
                tokenizer.consume();
                tokenizer.memory()[1] += 1;
                if tokenizer.memory()[1] == tokenizer.memory()[0] {
                    tokenizer.exit("balanced");
                    Step::Ok
                } else {
                    Step::Next(3)
                }
            }
            (1 | 2, Some(_)) => {
                tokenizer.consume();
                Step::Next(2)
            }
            _ => Step::Nok,
        }
    }

    for (input, expected) in [
        ("{{a}}", vec!["{{a}}"]),
        // Tried again at the next `{`.
        ("{{a}", vec!["{a}"]),
        ("{a}}", vec!["{a}"]),
        ("{a} {{b}}", vec!["{a}", "{{b}}"]),
    ] {
        let mut found = vec![];
        values(&to_mdast(input, &scripted(balanced)).unwrap(), &mut found);

        assert_eq!(
            found, expected,
            "should keep memory in a match, from zero, in {:?}",
            input
        );
    }
}

#[test]
fn reads_characters() {
    /// `:` and one alphabetic character.
    fn letter(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current_char()) {
            (0, Some(':')) => {
                tokenizer.enter("letter");
                tokenizer.consume();
                Step::Next(1)
            }
            (1, Some(char)) if char.is_alphabetic() => {
                tokenizer.consume();
                Step::Next(2)
            }
            // The rest of the character.
            (2, None) if tokenizer.current().is_some() => {
                tokenizer.consume();
                Step::Next(2)
            }
            (2, _) => {
                tokenizer.exit("letter");
                Step::Ok
            }
            _ => Step::Nok,
        }
    }

    let parse = ParseOptions {
        text_constructs: vec![Box::new(Scripted {
            marker: b':',
            step: letter,
        })],
        ..ParseOptions::default()
    };

    for (input, expected) in [
        (":é", vec![":é"]),
        (":a b", vec![":a"]),
        (":\u{1F600}", vec![]),
        (":1", vec![]),
    ] {
        let mut found = vec![];
        values(&to_mdast(input, &parse).unwrap(), &mut found);

        assert_eq!(
            found, expected,
            "should give whole characters, and `None` inside one, in {:?}",
            input
        );
    }
}

#[test]
fn sees_parse_options() {
    /// `{` and a byte, if GFM autolink literals are on.
    fn with_gfm(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current()) {
            (0, Some(b'{')) if tokenizer.options().constructs.gfm_autolink_literal => {
                tokenizer.enter("a");
                tokenizer.consume();
                Step::Next(1)
            }
            (1, Some(_)) => {
                tokenizer.consume();
                tokenizer.exit("a");
                Step::Ok
            }
            _ => Step::Nok,
        }
    }

    let mut found = vec![];
    values(&to_mdast("{x", &scripted(with_gfm)).unwrap(), &mut found);
    assert_eq!(found, Vec::<String>::new(), "should see options (off)");

    let parse = ParseOptions {
        constructs: markdown::Constructs::gfm(),
        ..scripted(with_gfm)
    };
    let mut found = vec![];
    values(&to_mdast("{x", &parse).unwrap(), &mut found);
    assert_eq!(found, vec!["{x"], "should see options (on)");
}

#[test]
fn errors_with_more_than_255_constructs() {
    let parse = options((0..256).map(|_| mention()).collect());
    let error = to_mdast("@a", &parse).unwrap_err();

    assert_eq!(
        error.to_string(),
        "Unexpected 256 constructs, expected at most 255 (markdown-rs:too-many-constructs)",
        "should error with 256 constructs"
    );
    assert!(
        to_mdast("@a", &options((0..255).map(|_| mention()).collect())).is_ok(),
        "should work with 255 constructs"
    );
}
