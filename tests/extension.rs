use markdown::{
    extension::{Construct, ConstructTokenizer, ContentType, Step, Token},
    mdast::{Blockquote, Custom, Node, Paragraph, Root, Text},
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
            children: tokens
                .into_iter()
                .flat_map(|token| token.children)
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
        (
            "`Attempt` after consuming",
            |state, tokenizer| match state {
                0 => {
                    tokenizer.enter("a");
                    tokenizer.consume();
                    Step::Attempt {
                        state: 10,
                        ok: 1,
                        nok: 1,
                    }
                }
                10 => Step::Ok,
                _ => {
                    tokenizer.exit("a");
                    Step::Ok
                }
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
        (
            "a token in text content after content on its line",
            |state, tokenizer| match state {
                0 => {
                    tokenizer.enter("a");
                    tokenizer.consume();
                    tokenizer.enter_content("b", ContentType::Text);
                    Step::Next(1)
                }
                1 => {
                    tokenizer.consume();
                    Step::Next(2)
                }
                _ => {
                    tokenizer.enter("mid");
                    tokenizer.consume();
                    tokenizer.exit("mid");
                    tokenizer.exit("b");
                    tokenizer.exit("a");
                    Step::Ok
                }
            },
        ),
        (
            "a content token in content",
            |state, tokenizer| match state {
                0 => {
                    tokenizer.enter("a");
                    tokenizer.consume();
                    tokenizer.enter_content("b", ContentType::Text);
                    Step::Next(1)
                }
                1 => {
                    tokenizer.consume();
                    Step::Next(2)
                }
                _ => {
                    tokenizer.enter_content("c", ContentType::Text);
                    tokenizer.consume();
                    tokenizer.exit("c");
                    tokenizer.exit("b");
                    tokenizer.exit("a");
                    Step::Ok
                }
            },
        ),
        (
            "closing, in an attempt, a token opened before it",
            |state, tokenizer| match state {
                0 => {
                    tokenizer.enter("outer");
                    tokenizer.enter("inner");
                    tokenizer.consume();
                    tokenizer.exit("inner");
                    Step::Next(1)
                }
                1 => Step::Attempt {
                    state: 10,
                    ok: 2,
                    nok: 2,
                },
                10 => {
                    tokenizer.exit("outer");
                    Step::Nok
                }
                _ => Step::Ok,
            },
        ),
        (
            "a wrong exit in an attempt, in content",
            |state, tokenizer| match state {
                0 => {
                    tokenizer.enter("a");
                    tokenizer.consume();
                    tokenizer.enter_content("b", ContentType::Text);
                    Step::Next(1)
                }
                1 => {
                    tokenizer.consume();
                    Step::Next(2)
                }
                2 => Step::Attempt {
                    state: 10,
                    ok: 3,
                    nok: 4,
                },
                3 => {
                    tokenizer.exit("b");
                    tokenizer.exit("a");
                    Step::Ok
                }
                10 => {
                    tokenizer.exit("nope");
                    Step::Ok
                }
                _ => Step::Nok,
            },
        ),
        (
            "an attempt that succeeds without progress, forever",
            |state, tokenizer| match state {
                0 => {
                    tokenizer.enter("a");
                    tokenizer.consume();
                    Step::Next(1)
                }
                1 => Step::Attempt {
                    state: 10,
                    ok: 1,
                    nok: 2,
                },
                10 => Step::Ok,
                _ => Step::Nok,
            },
        ),
        (
            "an attempt that fails after progress, forever",
            |state, tokenizer| match state {
                0 => {
                    tokenizer.enter("a");
                    tokenizer.consume();
                    Step::Next(1)
                }
                1 => Step::Attempt {
                    state: 10,
                    ok: 2,
                    nok: 1,
                },
                10 => {
                    tokenizer.enter("b");
                    tokenizer.consume();
                    tokenizer.exit("b");
                    Step::Next(11)
                }
                _ => Step::Nok,
            },
        ),
        (
            "an attempt that leaves a token open",
            |state, tokenizer| match state {
                0 => {
                    tokenizer.enter("a");
                    tokenizer.consume();
                    Step::Next(1)
                }
                1 => Step::Attempt {
                    state: 10,
                    ok: 2,
                    nok: 3,
                },
                10 => {
                    tokenizer.enter("b");
                    tokenizer.consume();
                    Step::Ok
                }
                2 => {
                    tokenizer.exit("b");
                    tokenizer.exit("a");
                    Step::Ok
                }
                _ => Step::Nok,
            },
        ),
        (
            "nested attempts that fail and are tried again",
            |state, tokenizer| {
                if state == 0 {
                    tokenizer.enter("a");
                    tokenizer.consume();
                    Step::Next(1)
                } else {
                    Step::Attempt {
                        state: 1,
                        ok: 1,
                        nok: 1,
                    }
                }
            },
        ),
        ("document content in text", |state, tokenizer| match state {
            0 => {
                tokenizer.enter("a");
                tokenizer.consume();
                tokenizer.enter_content("b", ContentType::Document);
                Step::Next(1)
            }
            _ => {
                tokenizer.consume();
                tokenizer.exit("b");
                tokenizer.exit("a");
                Step::Ok
            }
        }),
        // Its content, `{y`, does not match again: without the rule, the
        // construct matches.
        ("content at the first byte", |state, tokenizer| {
            match (state, tokenizer.current()) {
                (0, _) => {
                    tokenizer.enter("a");
                    tokenizer.enter_content("b", ContentType::Text);
                    tokenizer.consume();
                    Step::Next(1)
                }
                (1, _) => {
                    tokenizer.consume();
                    Step::Next(2)
                }
                (_, Some(b'z')) => {
                    tokenizer.exit("b");
                    tokenizer.consume();
                    tokenizer.exit("a");
                    Step::Ok
                }
                _ => Step::Nok,
            }
        }),
    ];

    for (label, step) in cases {
        let tree = to_mdast("x {yz", &scripted(step)).unwrap();
        assert!(
            find_scripted(&tree).is_none(),
            "should not match a construct that breaks a rule: {}",
            label
        );
        assert_eq!(
            tree.to_string(),
            "x {yz",
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
fn allows_a_retry_before_each_byte() {
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

    let parse = ParseOptions {
        flow_constructs: vec![Box::new(mention())],
        ..options((0..255).map(|_| mention()).collect())
    };
    assert!(
        to_mdast("@a", &parse).is_err(),
        "should count text and flow constructs together"
    );

    let parse = ParseOptions {
        document_constructs: vec![Box::new(mention())],
        ..options((0..255).map(|_| mention()).collect())
    };
    assert!(
        to_mdast("@a", &parse).is_err(),
        "should count containers too"
    );
}

/// `{{`, content parsed as text, `}}`.
fn braces_content(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (0, Some(b'{')) => {
            tokenizer.enter("braces");
            tokenizer.consume();
            Step::Next(1)
        }
        (1, Some(b'{')) => {
            tokenizer.consume();
            tokenizer.enter_content("bracesContent", ContentType::Text);
            Step::Next(2)
        }
        (2, Some(b'}')) => {
            tokenizer.exit("bracesContent");
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

fn scripted_flow(marker: u8, step: StepFn) -> ParseOptions {
    ParseOptions {
        flow_constructs: vec![Box::new(Scripted { marker, step })],
        ..ParseOptions::default()
    }
}

fn html(input: &str, parse: ParseOptions) -> String {
    to_html_with_options(
        input,
        &Options {
            parse,
            ..Options::default()
        },
    )
    .unwrap()
}

#[test]
fn parses_content_as_markdown() {
    let children = phrasing(to_mdast("a {{*b*}} c", &scripted(braces_content)).unwrap());
    let node = custom(&children[1]);

    assert!(
        matches!(node.children[..], [Node::Emphasis(_)]),
        "should give the content as children, got {:?}",
        node.children
    );
    assert_eq!(
        node.value.as_deref(),
        Some("{{}}"),
        "should leave content out of the values of other tokens"
    );
    assert_eq!(
        node.attributes.get("bracesContent").map(String::as_str),
        Some(""),
        "should give content tokens an empty value"
    );
}

#[test]
fn parses_content_across_lines_without_prefixes() {
    let tree = to_mdast("> a {{*b\n> c*}} d", &scripted(braces_content)).unwrap();
    let node = find_scripted(&tree).expect("expected a match");

    assert_eq!(
        node.children[0].to_string(),
        "b\nc",
        "should parse content across lines, without the `> ` prefix"
    );
    assert_eq!(
        node.children[0].position(),
        Some(&Position::new(1, 7, 6, 2, 5, 13)),
        "should give nested nodes positions in the source"
    );
}

#[test]
fn keeps_whitespace_around_content() {
    let tree = to_mdast("{{ *a* }}", &scripted(braces_content)).unwrap();

    assert_eq!(
        find_scripted(&tree).unwrap().children.len(),
        3,
        "should keep initial and final whitespace, like micromark labels"
    );
}

#[test]
fn parses_constructs_in_content() {
    let parse = ParseOptions {
        text_constructs: vec![
            Box::new(Scripted {
                marker: b'{',
                step: braces_content,
            }),
            Box::new(mention()),
        ],
        ..ParseOptions::default()
    };
    let tree = to_mdast("{{x @a y}}", &parse).unwrap();
    let node = find_scripted(&tree).expect("expected a match");

    assert!(
        matches!(&node.children[1], Node::Custom(Custom { name, .. }) if name == "mention"),
        "should parse other constructs in content, got {:?}",
        node.children
    );
}

#[test]
fn drops_empty_content() {
    let empty: StepFn = |state, tokenizer| match state {
        0 => {
            tokenizer.enter("a");
            tokenizer.consume();
            tokenizer.enter_content("b", ContentType::Text);
            Step::Next(1)
        }
        _ => {
            tokenizer.exit("b");
            tokenizer.exit("a");
            Step::Ok
        }
    };
    let tree = to_mdast("{", &scripted(empty)).unwrap();

    assert_eq!(
        find_scripted(&tree).and_then(|node| node.fields.get("tokens").cloned()),
        Some("a".into()),
        "should drop a content token without content"
    );
}

#[test]
fn to_html_renders_content_and_writes_own_text() {
    assert_eq!(
        html("a {{*b*}} c", scripted(braces_content)),
        "<p>a {{<em>b</em>}} c</p>",
        "should render content once, with the construct text around it"
    );
    assert_eq!(
        html("> a {{*b\n> c*}} d", scripted(braces_content)),
        "<blockquote>\n<p>a {{<em>b\nc</em>}} d</p>\n</blockquote>",
        "should write construct text without prefixes"
    );
}

/// `braces_content`, with a `|` token at the start of later lines.
fn braces_prefixed(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (2, Some(b'\n')) => {
            tokenizer.consume();
            Step::Next(4)
        }
        (4, Some(b'|')) => {
            tokenizer.enter("bracesPrefix");
            tokenizer.consume();
            tokenizer.exit("bracesPrefix");
            Step::Next(2)
        }
        (4, _) => Step::Retry(2),
        _ => braces_content(state, tokenizer),
    }
}

#[test]
fn keeps_tokens_at_line_starts_out_of_content() {
    let tree = to_mdast("{{*a\n|b*}}", &scripted(braces_prefixed)).unwrap();
    let node = find_scripted(&tree).expect("expected a match");

    assert_eq!(
        node.children[0].to_string(),
        "a\nb",
        "should parse content across the token, without it"
    );
    assert_eq!(
        node.fields.get("tokens").map(String::as_str),
        Some("braces,bracesContent"),
        "should not give tokens in content to `to_mdast`"
    );
    assert_eq!(
        html("{{*a\n|b*}}", scripted(braces_prefixed)),
        "<p>{{<em>a\nb</em>}}</p>",
        "should leave tokens in content out of HTML"
    );
}

#[test]
fn content_has_no_blank_lines() {
    // Built-in constructs in text expect no blank lines, as in a paragraph.
    for input in [
        "{{[x](\n}}",
        "{{\na}}",
        "{{ }}",
        "{{a\n|\n|b}}",
        "{{a\n| \n|b}}",
        "{{[a](b\n|\n|c)}}",
        "{{<a\n|\n|b>}}",
    ] {
        let tree = to_mdast(input, &scripted(braces_prefixed)).unwrap();

        assert!(
            find_scripted(&tree).is_none(),
            "should not match content with a blank line in {:?}",
            input
        );
        assert_eq!(
            html(input, scripted(braces_prefixed)),
            to_html_with_options(input, &Options::default()).unwrap(),
            "should keep text for content with a blank line in {:?}",
            input
        );
    }
}

#[test]
fn tokens_do_not_end_inside_a_tab() {
    /// `>` and the next three bytes, which can end inside a tab.
    fn four_bytes(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match state {
            0 => {
                tokenizer.enter("a");
                tokenizer.consume();
                Step::Next(1)
            }
            1..=3 => {
                tokenizer.consume();
                Step::Next(state + 1)
            }
            _ => {
                tokenizer.exit("a");
                Step::Ok
            }
        }
    }

    /// `>` and the whitespace after it, character by character.
    fn whitespace(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current_char()) {
            (0, _) => {
                tokenizer.enter("a");
                tokenizer.consume();
                Step::Next(1)
            }
            // Columns of a tab after its first.
            (1, None) if tokenizer.current().is_some() => {
                tokenizer.consume();
                Step::Next(1)
            }
            (1, Some(' ' | '\t')) => {
                tokenizer.consume();
                Step::Next(1)
            }
            _ => {
                tokenizer.exit("a");
                Step::Ok
            }
        }
    }

    /// `>` and one tab, read as a character.
    fn tab(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current_char()) {
            (0, _) => {
                tokenizer.enter("a");
                tokenizer.consume();
                Step::Next(1)
            }
            (1, Some('\t')) => {
                tokenizer.consume();
                Step::Next(2)
            }
            (2, None) if tokenizer.current().is_some() => {
                tokenizer.consume();
                Step::Next(2)
            }
            (2, _) => {
                tokenizer.exit("a");
                Step::Ok
            }
            _ => Step::Nok,
        }
    }

    let gfm = |step| ParseOptions {
        text_constructs: vec![Box::new(Scripted { marker: b'>', step })],
        ..ParseOptions::gfm()
    };

    let tree = to_mdast("#>\t z", &gfm(tab)).unwrap();
    assert_eq!(
        custom(&tree.children().unwrap()[0].children().unwrap()[1])
            .value
            .as_deref(),
        Some(">\t"),
        "should give `None` as the character inside a tab"
    );

    /// The rest of a tab that a line starts inside of.
    fn rest_of_tab(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current_char()) {
            (0, Some(' ')) => {
                tokenizer.enter("a");
                tokenizer.consume();
                Step::Next(1)
            }
            (1, None) if tokenizer.current().is_some() => {
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

    let parse = ParseOptions {
        text_constructs: vec![Box::new(Scripted {
            marker: b' ',
            step: rest_of_tab,
        })],
        ..ParseOptions::default()
    };
    assert!(
        find_scripted(&to_mdast("- a\n\t@b", &parse).unwrap()).is_some(),
        "should start a token where a line starts inside a tab"
    );

    let input = "#>  \tza@b.c";

    assert_eq!(
        to_mdast(input, &gfm(four_bytes)).unwrap(),
        to_mdast(input, &ParseOptions::gfm()).unwrap(),
        "should not match a construct that ends inside a tab"
    );

    let tree = to_mdast(input, &gfm(whitespace)).unwrap();
    let paragraph = tree.children().unwrap()[0].children().unwrap();

    assert_eq!(
        custom(&paragraph[1]).value.as_deref(),
        Some(">  \t"),
        "should match a construct that takes the whole tab"
    );
    assert_eq!(
        paragraph[2].position(),
        Some(&Position::new(1, 9, 5, 1, 15, 11)),
        "should keep positions after the tab"
    );
}

#[test]
fn ends_gfm_autolink_literals_at_the_end_of_content() {
    /// `{`, content up to `X`, `X`, `}`.
    fn up_to_x(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current()) {
            (0, Some(b'{')) => {
                tokenizer.enter("a");
                tokenizer.consume();
                tokenizer.enter_content("b", ContentType::Text);
                Step::Next(1)
            }
            (1, Some(b'X')) => {
                tokenizer.exit("b");
                tokenizer.consume();
                Step::Next(2)
            }
            (1, Some(_)) => {
                tokenizer.consume();
                Step::Next(1)
            }
            (2, Some(b'}')) => {
                tokenizer.consume();
                tokenizer.exit("a");
                Step::Ok
            }
            _ => Step::Nok,
        }
    }

    let parse = ParseOptions {
        text_constructs: vec![Box::new(Scripted {
            marker: b'{',
            step: up_to_x,
        })],
        ..ParseOptions::gfm()
    };
    let tree = to_mdast("{www.aX}", &parse).unwrap();
    let node = find_scripted(&tree).expect("expected a match");

    assert!(
        matches!(&node.children[..], [Node::Link(link)] if link.children[0].to_string() == "www.a"),
        "should end a literal where the content ends, got {:?}",
        node.children
    );
}

#[test]
fn errors_for_mdx_jsx_left_open_in_content() {
    let parse = ParseOptions {
        constructs: markdown::Constructs::mdx(),
        ..scripted(braces_content)
    };

    assert_eq!(
        to_mdast("a {{<b>}} c", &parse)
            .unwrap_err()
            .rule_id
            .as_str(),
        "end-tag-mismatch",
        "should error like a paragraph does"
    );
}

#[test]
fn errors_for_mdx_jsx_closed_in_other_content() {
    let parse = ParseOptions {
        constructs: markdown::Constructs::mdx(),
        ..scripted(braces_content)
    };

    assert_eq!(
        to_mdast("x <a> {{b</a>}} c", &parse)
            .unwrap_err()
            .rule_id
            .as_str(),
        "end-tag-mismatch",
        "should error for a closing tag whose opening tag is outside the content"
    );
}

/// Levels of matches of `scripted` in `node`.
fn depth(node: &Node) -> usize {
    let own = usize::from(matches!(node, Node::Custom(custom) if custom.name == "scripted"));
    own + node
        .children()
        .map_or(0, |children| children.iter().map(depth).max().unwrap_or(0))
}

/// `(`, content with balanced parentheses, `)`.
fn parens(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (0, Some(b'(')) => {
            tokenizer.enter("a");
            tokenizer.consume();
            tokenizer.enter_content("b", ContentType::Text);
            tokenizer.memory()[0] = 1;
            Step::Next(1)
        }
        (1, Some(b')')) if tokenizer.memory()[0] == 1 => {
            tokenizer.exit("b");
            tokenizer.consume();
            tokenizer.exit("a");
            Step::Ok
        }
        (1, Some(byte)) => {
            match byte {
                b'(' => tokenizer.memory()[0] += 1,
                b')' => tokenizer.memory()[0] -= 1,
                _ => {}
            }
            tokenizer.consume();
            Step::Next(1)
        }
        _ => Step::Nok,
    }
}

#[test]
fn limits_the_depth_of_content() {
    let parse = ParseOptions {
        text_constructs: vec![Box::new(Scripted {
            marker: b'(',
            step: parens,
        })],
        ..ParseOptions::default()
    };
    let depth_of = |levels: usize| {
        let input = format!("{}a{}", "(".repeat(levels), ")".repeat(levels));
        depth(&to_mdast(&input, &parse).unwrap())
    };
    // With the text of the paragraph, 31 levels are 32 levels of content.
    assert_eq!(depth_of(31), 31, "should nest content 32 deep");
    assert_eq!(
        depth_of(40),
        31,
        "should not match content deeper than that"
    );
}

/// `{`, content, `,`, content, `}`, with a `|` token allowed at the start of
/// the second content.
fn pair(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (0, Some(b'{')) => {
            tokenizer.enter("pair");
            tokenizer.consume();
            tokenizer.enter_content("left", ContentType::Text);
            Step::Next(1)
        }
        (1, Some(b',')) => {
            tokenizer.exit("left");
            tokenizer.consume();
            tokenizer.enter_content("right", ContentType::Text);
            tokenizer.memory()[0] = 1;
            Step::Next(2)
        }
        (2, Some(b'|')) if tokenizer.memory()[0] == 1 => {
            tokenizer.enter("pairPrefix");
            tokenizer.consume();
            tokenizer.exit("pairPrefix");
            tokenizer.memory()[0] = 0;
            Step::Next(2)
        }
        (2, Some(b'}')) => {
            tokenizer.exit("right");
            tokenizer.consume();
            tokenizer.exit("pair");
            Step::Ok
        }
        (1 | 2, Some(_)) => {
            tokenizer.consume();
            tokenizer.memory()[0] = 0;
            Step::Next(state)
        }
        _ => Step::Nok,
    }
}

#[test]
fn parses_each_content_token_alone() {
    let tree = to_mdast("{*a*,*b*}", &scripted(pair)).unwrap();
    let node = find_scripted(&tree).expect("expected a match");

    assert!(
        matches!(node.children[..], [Node::Emphasis(_), Node::Emphasis(_)]),
        "should parse each content token on its own, got {:?}",
        node.children
    );
    assert_eq!(
        html("{*a*,*b*}", scripted(pair)),
        "<p>{<em>a</em>,<em>b</em>}</p>",
        "should render each content token"
    );

    let tree = to_mdast("{a,|b}", &scripted(pair)).unwrap();
    let node = find_scripted(&tree).expect("expected a match");

    assert_eq!(
        node.fields.get("tokens").map(String::as_str),
        Some("pair,left,right"),
        "should allow a token before the content of a later content token"
    );
}

#[test]
fn skips_prefix_tokens_of_an_outer_match() {
    /// `braces_prefixed`, or `{[`, raw text as a token, `]]`.
    fn outer_or_raw(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current()) {
            (1, Some(b'[')) => {
                tokenizer.consume();
                tokenizer.enter("raw");
                Step::Next(20)
            }
            (20, Some(b']')) => {
                tokenizer.exit("raw");
                tokenizer.consume();
                Step::Next(21)
            }
            (20, Some(_)) => {
                tokenizer.consume();
                Step::Next(20)
            }
            (21, Some(b']')) => {
                tokenizer.consume();
                tokenizer.exit("braces");
                Step::Ok
            }
            _ => braces_prefixed(state, tokenizer),
        }
    }

    let tree = to_mdast("{{a {[b\n|c]] d}}", &scripted(outer_or_raw)).unwrap();
    let outer = find_scripted(&tree).expect("expected a match");
    let inner = outer
        .children
        .iter()
        .find_map(find_scripted)
        .expect("expected an inner match");

    assert_eq!(
        inner.attributes.get("raw").map(String::as_str),
        Some("b\nc"),
        "should leave the prefix of the outer match out of the inner one"
    );
    assert_eq!(
        inner.fields.get("tokens").map(String::as_str),
        Some("braces,raw"),
        "should not give the prefix of the outer match to the inner one"
    );
}

#[test]
fn undoes_a_failed_attempt() {
    let optional: StepFn = |state, tokenizer| match (state, tokenizer.current()) {
        (0, Some(b'{')) => {
            tokenizer.enter("a");
            tokenizer.consume();
            Step::Next(1)
        }
        (1, _) => Step::Attempt {
            state: 10,
            ok: 2,
            nok: 2,
        },
        (10, Some(b'x')) => {
            tokenizer.enter("optional");
            tokenizer.consume();
            Step::Next(11)
        }
        (11, Some(b'y')) => {
            tokenizer.consume();
            tokenizer.exit("optional");
            Step::Ok
        }
        (2, Some(b'}')) => {
            tokenizer.consume();
            tokenizer.exit("a");
            Step::Ok
        }
        (2, Some(_)) => {
            tokenizer.consume();
            Step::Next(2)
        }
        _ => Step::Nok,
    };
    let tokens = |input| {
        let tree = to_mdast(input, &scripted(optional)).unwrap();
        find_scripted(&tree).and_then(|node| node.fields.get("tokens").cloned())
    };

    assert_eq!(
        tokens("{xy}"),
        Some("a,optional".into()),
        "should keep a match"
    );
    assert_eq!(
        tokens("{xz}"),
        Some("a".into()),
        "should undo a failure, and continue at `nok`"
    );
}

#[test]
fn undoes_a_failed_attempt_that_ends_a_line_of_content() {
    let step: StepFn = |state, tokenizer| match (state, tokenizer.current()) {
        (0, Some(b'{')) => {
            tokenizer.enter("a");
            tokenizer.consume();
            tokenizer.enter_content("b", ContentType::Text);
            Step::Next(1)
        }
        (1, Some(b'x')) => {
            tokenizer.consume();
            Step::Next(2)
        }
        (2, _) => Step::Attempt {
            state: 10,
            ok: 3,
            nok: 3,
        },
        (10, Some(b'\n')) => {
            tokenizer.consume();
            Step::Next(11)
        }
        (3, Some(b'}')) => {
            tokenizer.exit("b");
            tokenizer.consume();
            tokenizer.exit("a");
            Step::Ok
        }
        (3, Some(_)) => {
            tokenizer.consume();
            Step::Next(3)
        }
        _ => Step::Nok,
    };
    let tree = to_mdast("{x\ny}", &scripted(step)).unwrap();

    assert_eq!(
        find_scripted(&tree).map(|node| Node::Paragraph(Paragraph {
            children: node.children.clone(),
            position: None
        })
        .to_string()),
        Some("x\ny".into()),
        "should keep content before an attempt that consumed a line ending"
    );
}

#[test]
fn keeps_content_across_attempts_in_it() {
    // `{`, content, and an attempt at `y`, then `}`.
    let step: StepFn = |state, tokenizer| match (state, tokenizer.current()) {
        (0, Some(b'{')) => {
            tokenizer.enter("a");
            tokenizer.consume();
            tokenizer.enter_content("b", ContentType::Text);
            Step::Next(1)
        }
        (1, Some(b'x')) => {
            tokenizer.consume();
            Step::Next(2)
        }
        (2, _) => Step::Attempt {
            state: 10,
            ok: 3,
            nok: 3,
        },
        // Content up to `y`, which succeeds, or `}`, which fails.
        (10, Some(b'y')) => {
            tokenizer.consume();
            Step::Next(11)
        }
        (10, Some(byte)) if byte != b'}' => {
            tokenizer.consume();
            Step::Next(10)
        }
        (11, _) => Step::Ok,
        (3, _) => {
            tokenizer.exit("b");
            Step::Retry(4)
        }
        (4, Some(b'}')) => {
            tokenizer.consume();
            tokenizer.exit("a");
            Step::Ok
        }
        (4, Some(_)) => {
            tokenizer.consume();
            Step::Next(4)
        }
        _ => Step::Nok,
    };
    let content = |input| {
        let tree = to_mdast(input, &scripted(step)).unwrap();
        find_scripted(&tree).map(|node| {
            Node::Paragraph(Paragraph {
                children: node.children.clone(),
                position: None,
            })
            .to_string()
        })
    };

    assert_eq!(
        content("{xy}"),
        Some("xy".into()),
        "should keep content that an attempt added"
    );
    assert_eq!(
        content("{x\nz}"),
        Some("x".into()),
        "should undo a line of content that a failed attempt added"
    );
}

#[test]
fn allows_an_attempt_before_each_byte() {
    /// `{`, then `a`s, each consumed in an attempt.
    fn attempt_each(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current()) {
            (0, Some(b'{')) => {
                tokenizer.enter("a");
                tokenizer.consume();
                Step::Next(1)
            }
            (1, Some(b'a')) => Step::Attempt {
                state: 10,
                ok: 1,
                nok: 2,
            },
            (10, Some(b'a')) => {
                tokenizer.enter("b");
                tokenizer.consume();
                tokenizer.exit("b");
                Step::Next(11)
            }
            (11, _) => Step::Ok,
            (1 | 2, _) => {
                tokenizer.exit("a");
                Step::Ok
            }
            _ => Step::Nok,
        }
    }

    let input = format!("{{{}", "a".repeat(300));
    let mut found = vec![];
    values(
        &to_mdast(&input, &scripted(attempt_each)).unwrap(),
        &mut found,
    );

    assert_eq!(
        found,
        vec![input],
        "should allow an attempt before each byte of a long match"
    );
}

#[test]
fn restores_content_after_a_failed_attempt() {
    /// `{`, content of `x` and a line ending, where an attempt tries to
    /// take everything up to `}` and fails there, `}`; a `|` token can
    /// start a line of content.
    fn step(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current()) {
            (0, Some(b'{')) => {
                tokenizer.enter("a");
                tokenizer.consume();
                tokenizer.enter_content("b", ContentType::Text);
                Step::Next(1)
            }
            (1, Some(b'x')) => {
                tokenizer.consume();
                Step::Next(5)
            }
            (5, Some(b'\n')) => {
                tokenizer.consume();
                Step::Next(2)
            }
            (2, _) => Step::Attempt {
                state: 10,
                ok: 4,
                nok: 4,
            },
            (10, Some(byte)) if byte != b'}' => {
                tokenizer.consume();
                Step::Next(10)
            }
            (3, Some(b'\n')) => {
                tokenizer.consume();
                Step::Next(4)
            }
            (4, Some(b'|')) => {
                tokenizer.enter("prefix");
                tokenizer.consume();
                tokenizer.exit("prefix");
                Step::Next(3)
            }
            (3 | 4, Some(b'}')) => {
                tokenizer.exit("b");
                tokenizer.consume();
                tokenizer.exit("a");
                Step::Ok
            }
            (3 | 4, Some(_)) => {
                tokenizer.consume();
                Step::Next(3)
            }
            _ => Step::Nok,
        }
    }

    for (input, expected) in [
        // New chunks after the undo link to the chunk before the attempt.
        ("{x\nz\nw}", "x\nz\nw"),
        // The line of the last content is that before the attempt.
        ("{x\n|z}", "x\nz"),
    ] {
        let tree = to_mdast(input, &scripted(step)).unwrap();
        let node = find_scripted(&tree).expect("expected a match");

        assert_eq!(
            Node::Paragraph(Paragraph {
                children: node.children.clone(),
                position: None
            })
            .to_string(),
            expected,
            "should continue content after a failed attempt in {:?}",
            input
        );
    }
}

/// `{` and `a`s, then `N` retries.
fn spend<const N: usize>(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (0, _) => {
            tokenizer.enter("a");
            tokenizer.consume();
            Step::Next(1)
        }
        (1, Some(b'a')) => {
            tokenizer.consume();
            Step::Next(1)
        }
        _ if tokenizer.memory()[0] < N => {
            tokenizer.memory()[0] += 1;
            Step::Retry(2)
        }
        _ => {
            tokenizer.exit("a");
            Step::Ok
        }
    }
}

#[test]
fn limits_steps_per_byte_reached() {
    // 10 bytes and the end allow 2,816 steps: 11 to consume and end, and
    // 2,805 retries.
    let mut found = vec![];
    values(
        &to_mdast("{aaaaaaaaa", &scripted(spend::<2805>)).unwrap(),
        &mut found,
    );
    assert_eq!(
        found,
        vec!["{aaaaaaaaa"],
        "should allow steps for each byte reached, anywhere in the match"
    );

    let mut found = vec![];
    values(
        &to_mdast(
            &format!("{{aaaaaaaaa{}{{aaaaaaaaa", " b".repeat(500)),
            &scripted(spend::<2000>),
        )
        .unwrap(),
        &mut found,
    );
    assert_eq!(
        found,
        vec!["{aaaaaaaaa", "{aaaaaaaaa"],
        "should count steps for each match, from its start"
    );

    let mut found = vec![];
    values(
        &to_mdast("{aaaaaaaaa", &scripted(spend::<2806>)).unwrap(),
        &mut found,
    );
    assert_eq!(
        found,
        Vec::<String>::new(),
        "should not match past 256 steps for each byte reached"
    );

    // Before `{`, an attempt looks at every byte and fails; elsewhere, 1,000
    // retries, which one byte does not allow.
    let look_or_spend: StepFn = |state, tokenizer| match (state, tokenizer.current()) {
        (0, _) => {
            tokenizer.enter("a");
            tokenizer.consume();
            Step::Next(1)
        }
        (1, Some(b'{')) => Step::Attempt {
            state: 10,
            ok: 2,
            nok: 2,
        },
        (10, Some(_)) => {
            tokenizer.enter("b");
            tokenizer.consume();
            tokenizer.exit("b");
            Step::Next(10)
        }
        (1 | 2, _) if tokenizer.memory()[0] < 1000 => {
            tokenizer.memory()[0] += 1;
            Step::Retry(2)
        }
        (1 | 2, _) => {
            tokenizer.exit("a");
            Step::Ok
        }
        _ => Step::Nok,
    };
    let input = format!("{{{{{}", "a".repeat(20));

    let mut found = vec![];
    values(
        &to_mdast(&input, &scripted(look_or_spend)).unwrap(),
        &mut found,
    );
    assert_eq!(
        found,
        vec!["{"],
        "should count bytes a failed attempt reached, but not in a later match"
    );
}

#[test]
fn restores_the_line_start_after_a_failed_attempt() {
    /// `{a`, a line ending, and an attempt that takes the next line and
    /// fails.
    fn step(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current()) {
            (0, _) => {
                tokenizer.enter("a");
                tokenizer.consume();
                Step::Next(1)
            }
            (1 | 2 | 10 | 11, Some(_)) => {
                tokenizer.consume();
                Step::Next(state + 1)
            }
            (3, _) => Step::Attempt {
                state: 10,
                ok: 4,
                nok: 4,
            },
            (4, _) => {
                tokenizer.exit("a");
                Step::Ok
            }
            _ => Step::Nok,
        }
    }

    let tree = to_mdast("{a\nb\nc", &scripted(step)).unwrap();
    assert_eq!(
        find_scripted(&tree).and_then(|node| node.value.as_deref()),
        Some("{a\n"),
        "should end the token where the attempt started"
    );
    assert_eq!(
        html("{a\nb\nc", scripted(step)),
        "<p>{a\nb\nc</p>",
        "should continue text where the attempt started"
    );
}

#[test]
fn forgets_the_content_of_an_earlier_match() {
    /// `{`, then content of `c` and failure, or a failed attempt and `Ok`.
    fn step(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current()) {
            (0, _) => {
                tokenizer.enter("a");
                tokenizer.consume();
                Step::Next(1)
            }
            (1, Some(b'c')) => {
                tokenizer.enter_content("b", ContentType::Text);
                tokenizer.consume();
                Step::Next(2)
            }
            (1, _) => Step::Attempt {
                state: 10,
                ok: 3,
                nok: 3,
            },
            (3, _) => {
                tokenizer.exit("a");
                Step::Ok
            }
            _ => Step::Nok,
        }
    }

    let mut found = vec![];
    values(&to_mdast("{c {x", &scripted(step)).unwrap(), &mut found);
    assert_eq!(found, vec!["{"], "should match after an earlier failure");
}

#[test]
fn fails_only_the_attempt_that_breaks_a_rule() {
    let step: StepFn = |state, tokenizer| match state {
        0 => {
            tokenizer.enter("a");
            tokenizer.consume();
            Step::Next(1)
        }
        1 => Step::Attempt {
            state: 10,
            ok: 2,
            nok: 3,
        },
        10 => {
            tokenizer.exit("nope");
            Step::Ok
        }
        3 => {
            tokenizer.exit("a");
            Step::Ok
        }
        _ => Step::Nok,
    };
    let mut found = vec![];
    values(&to_mdast("{x", &scripted(step)).unwrap(), &mut found);

    assert_eq!(found, vec!["{"], "should continue at `nok`");
}

#[test]
fn nests_attempts() {
    // The inner attempt fails, the outer one succeeds.
    let inner_fails: StepFn = |state, tokenizer| match state {
        0 => {
            tokenizer.enter("a");
            tokenizer.consume();
            Step::Next(1)
        }
        1 => Step::Attempt {
            state: 10,
            ok: 2,
            nok: 3,
        },
        10 => {
            tokenizer.enter("b");
            tokenizer.consume();
            Step::Next(11)
        }
        11 => Step::Attempt {
            state: 20,
            ok: 3,
            nok: 12,
        },
        20 => {
            tokenizer.consume();
            Step::Next(21)
        }
        12 => {
            tokenizer.exit("b");
            Step::Ok
        }
        2 => {
            tokenizer.exit("a");
            Step::Ok
        }
        _ => Step::Nok,
    };
    let tree = to_mdast("{xy", &scripted(inner_fails)).unwrap();
    let node = find_scripted(&tree).expect("expected a match");

    assert_eq!(
        (
            node.value.as_deref(),
            node.fields.get("tokens").map(String::as_str)
        ),
        (Some("{x"), Some("a,b")),
        "should undo an inner attempt and keep the outer one"
    );

    // The inner attempt adds content, the outer one fails.
    let outer_fails: StepFn = |state, tokenizer| match state {
        0 => {
            tokenizer.enter("a");
            tokenizer.consume();
            tokenizer.enter_content("b", ContentType::Text);
            Step::Next(1)
        }
        1 => Step::Attempt {
            state: 10,
            ok: 2,
            nok: 2,
        },
        10 => Step::Attempt {
            state: 20,
            ok: 11,
            nok: 11,
        },
        20 => {
            tokenizer.consume();
            Step::Next(21)
        }
        21 => Step::Ok,
        11 => {
            tokenizer.consume();
            Step::Next(12)
        }
        2 => {
            tokenizer.consume();
            Step::Next(3)
        }
        3 => {
            tokenizer.exit("b");
            tokenizer.exit("a");
            Step::Ok
        }
        _ => Step::Nok,
    };
    let tree = to_mdast("{xy}", &scripted(outer_fails)).unwrap();
    let node = find_scripted(&tree).expect("expected a match");

    assert_eq!(
        (node.value.as_deref(), node.children[0].to_string()),
        (Some("{"), "x".into()),
        "should undo content of an inner attempt with the outer one"
    );
}

/// `{x`, `N` attempts that cross the line ending and fail, then the line
/// ending and `y`.
fn recross<const N: usize>(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (0, _) => {
            tokenizer.enter("a");
            tokenizer.consume();
            Step::Next(1)
        }
        (1, Some(b'x')) => {
            tokenizer.consume();
            Step::Next(2)
        }
        (2, Some(b'\n')) if tokenizer.memory()[0] < N => {
            tokenizer.memory()[0] += 1;
            Step::Attempt {
                state: 10,
                ok: 3,
                nok: 2,
            }
        }
        (2 | 10, Some(b'\n')) => {
            tokenizer.consume();
            Step::Next(state + 1)
        }
        (3, Some(b'y')) => {
            tokenizer.consume();
            tokenizer.exit("a");
            Step::Ok
        }
        _ => Step::Nok,
    }
}

#[test]
fn counts_container_prefixes_it_moves_past() {
    let prefix = ">".repeat(1000);

    assert!(
        find_scripted(&to_mdast("{x\ny", &scripted(recross::<300>)).unwrap()).is_some(),
        "should allow 300 attempts across a line ending"
    );
    assert!(
        find_scripted(
            &to_mdast(
                &format!("{} {{x\n{} y", prefix, prefix),
                &scripted(recross::<300>)
            )
            .unwrap()
        )
        .is_none(),
        "should count the bytes of a container prefix that each attempt moves past"
    );
}

/// `{` and `a`s, then `N` attempts, each in the one before it.
fn nest<const N: usize>(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (0, _) => {
            tokenizer.enter("a");
            tokenizer.consume();
            Step::Next(1)
        }
        (1, Some(b'a')) => {
            tokenizer.consume();
            Step::Next(1)
        }
        (1 | 2, _) if tokenizer.memory()[0] < N => {
            tokenizer.memory()[0] += 1;
            tokenizer.memory()[1] += 1;
            Step::Attempt {
                state: 2,
                ok: 3,
                nok: 4,
            }
        }
        // The innermost attempt, and each one around it, succeeds.
        (1..=3, _) => {
            if state == 3 {
                tokenizer.memory()[1] -= 1;
            }
            if tokenizer.memory()[1] == 0 {
                tokenizer.exit("a");
            }
            Step::Ok
        }
        _ => Step::Nok,
    }
}

#[test]
fn limits_attempts_in_attempts() {
    let input = format!("{{{}", "a".repeat(20));

    assert!(
        find_scripted(&to_mdast(&input, &scripted(nest::<256>)).unwrap()).is_some(),
        "should allow 256 attempts in each other"
    );
    assert!(
        find_scripted(&to_mdast(&input, &scripted(nest::<257>)).unwrap()).is_none(),
        "should fail an attempt nested more than 256 deep"
    );
}

/// `::name[label]`, with the label parsed as text.
fn leaf(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (0 | 1, Some(b':')) => {
            if state == 0 {
                tokenizer.enter("leaf");
            }
            tokenizer.consume();
            Step::Next(state + 1)
        }
        (2 | 3, Some(b'a'..=b'z')) => {
            if state == 2 {
                tokenizer.enter("leafName");
            }
            tokenizer.consume();
            Step::Next(3)
        }
        (3, Some(b'[')) => {
            tokenizer.exit("leafName");
            tokenizer.consume();
            Step::Next(4)
        }
        (4, Some(b']')) => {
            tokenizer.consume();
            Step::Next(6)
        }
        (4, Some(_)) => {
            tokenizer.enter_content("leafLabel", ContentType::Text);
            Step::Retry(5)
        }
        (5, Some(b']')) => {
            tokenizer.exit("leafLabel");
            tokenizer.consume();
            Step::Next(6)
        }
        (5, Some(byte)) if byte != b'\n' => {
            tokenizer.consume();
            Step::Next(5)
        }
        (6, None | Some(b'\n')) => {
            tokenizer.exit("leaf");
            Step::Ok
        }
        _ => Step::Nok,
    }
}

#[test]
fn flow_construct_takes_a_line() {
    let tree = to_mdast("::a[*b*]", &scripted_flow(b':', leaf)).unwrap();
    assert!(
        matches!(&tree.children().unwrap()[0], Node::Custom(node) if matches!(node.children[..], [Node::Emphasis(_)])),
        "should make a block with a parsed label, got {:?}",
        tree
    );

    let tree = to_mdast("x\n::a[b]\ny", &scripted_flow(b':', leaf)).unwrap();
    assert_eq!(
        tree.children().unwrap().len(),
        3,
        "should interrupt a paragraph"
    );

    let tree = to_mdast("x\n::a[b]\n[c]: d", &scripted_flow(b':', leaf)).unwrap();
    assert!(
        matches!(
            &tree.children().unwrap()[..],
            [Node::Paragraph(_), Node::Custom(_), Node::Definition(_)]
        ),
        "should let a definition follow, got {:?}",
        tree
    );

    let parse = ParseOptions {
        constructs: markdown::Constructs::mdx(),
        mdx_esm_parse: Some(Box::new(|_| markdown::MdxSignal::Ok)),
        ..scripted_flow(b':', leaf)
    };
    let tree = to_mdast("x\n::a[b]\nimport a from 'b'", &parse).unwrap();
    assert!(
        matches!(
            &tree.children().unwrap()[..],
            [Node::Paragraph(_), Node::Custom(_), Node::MdxjsEsm(_)]
        ),
        "should let flow that cannot interrupt a paragraph follow, got {:?}",
        tree
    );

    let tree = to_mdast("> ::a[b]\n> c", &scripted_flow(b':', leaf)).unwrap();
    assert!(
        matches!(
            &tree.children().unwrap()[0].children().unwrap()[0],
            Node::Custom(_)
        ),
        "should work in containers"
    );

    for input in ["::a[b] x", "::a[b\nc]"] {
        let tree = to_mdast(input, &scripted_flow(b':', leaf)).unwrap();
        assert!(
            find_scripted(&tree).is_none(),
            "should not match a construct that does not end at a line ending: {:?}",
            input
        );
    }
}

#[test]
fn flow_construct_skips_indentation() {
    let tree = to_mdast("   ::a[b]", &scripted_flow(b':', leaf)).unwrap();
    assert!(
        find_scripted(&tree).is_some(),
        "should match after up to 3 columns"
    );

    let tree = to_mdast("    ::a[b]", &scripted_flow(b':', leaf)).unwrap();
    assert!(
        matches!(&tree.children().unwrap()[0], Node::Code(_)),
        "should leave 4 columns to indented code"
    );

    let any: StepFn = |state, tokenizer| match (state, tokenizer.current()) {
        (0, _) => {
            tokenizer.enter("a");
            tokenizer.consume();
            Step::Next(1)
        }
        (1, None | Some(b'\n')) => {
            tokenizer.exit("a");
            Step::Ok
        }
        _ => {
            tokenizer.consume();
            Step::Next(1)
        }
    };
    assert!(
        find_scripted(&to_mdast("  b", &scripted_flow(b':', any)).unwrap()).is_none(),
        "should not start after indentation without a marker"
    );

    for input in ["  b", "    b"] {
        let tree = to_mdast(input, &scripted_flow(b' ', any)).unwrap();
        assert!(
            find_scripted(&tree).is_none(),
            "should not take a space as a marker, which is indentation, in {:?}",
            input
        );
    }
}

#[test]
fn errors_when_a_flow_construct_fails_after_its_first_line() {
    let late: StepFn = |state, tokenizer| match (state, tokenizer.current()) {
        (0, Some(b'%')) => {
            tokenizer.enter("late");
            tokenizer.consume();
            Step::Next(1)
        }
        (1, Some(b'\n')) => {
            tokenizer.consume();
            Step::Next(2)
        }
        (1, Some(_)) => {
            tokenizer.consume();
            Step::Next(1)
        }
        _ => Step::Nok,
    };

    let message = to_mdast("%a\nb", &scripted_flow(b'%', late)).unwrap_err();
    assert_eq!(message.rule_id.as_str(), "flow-construct-late-failure");
    assert!(
        to_mdast("%a", &scripted_flow(b'%', late)).is_ok(),
        "should not error for a failure on the first line"
    );
}

#[test]
fn attempts_in_flow_stay_on_one_line() {
    let across: StepFn = |state, tokenizer| match (state, tokenizer.current()) {
        (0, Some(b'%')) => {
            tokenizer.enter("f");
            tokenizer.consume();
            Step::Next(1)
        }
        (1, _) => Step::Attempt {
            state: 10,
            ok: 2,
            nok: 2,
        },
        (10, Some(b'\n')) => {
            tokenizer.enter("x");
            tokenizer.consume();
            Step::Next(11)
        }
        (11, _) => {
            tokenizer.exit("x");
            Step::Ok
        }
        (2, None | Some(b'\n')) => {
            tokenizer.exit("f");
            Step::Ok
        }
        _ => Step::Nok,
    };
    let tree = to_mdast("%\nb", &scripted_flow(b'%', across)).unwrap();

    assert!(
        matches!(
            &tree.children().unwrap()[..],
            [Node::Custom(_), Node::Paragraph(_)]
        ),
        "should fail an attempt that consumes a line ending, got {:?}",
        tree
    );

    let across_then_fail: StepFn = |state, tokenizer| match (state, tokenizer.current()) {
        (0, Some(b'%')) => {
            tokenizer.enter("f");
            tokenizer.consume();
            Step::Next(1)
        }
        (1, _) => Step::Attempt {
            state: 10,
            ok: 2,
            nok: 2,
        },
        (10, Some(b'\n')) => {
            tokenizer.enter("x");
            tokenizer.consume();
            Step::Next(11)
        }
        _ => Step::Nok,
    };
    assert!(
        find_scripted(&to_mdast("%\nb", &scripted_flow(b'%', across_then_fail)).unwrap()).is_none(),
        "should fail on the first line after an attempt that consumes a line ending"
    );
}

/// `%%%`, raw lines as tokens, and `%%%` or the end: a fenced raw block.
fn fenced(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (0, Some(b'%')) => {
            tokenizer.enter("fenced");
            tokenizer.enter("fence");
            tokenizer.consume();
            Step::Next(1)
        }
        (1 | 2, Some(b'%')) => {
            tokenizer.consume();
            Step::Next(state + 1)
        }
        (3, Some(b'\n')) => {
            tokenizer.exit("fence");
            tokenizer.consume();
            Step::Next(4)
        }
        // At the start of a line, a closing fence, which is tried, or a
        // line.
        (4, Some(b'%')) => Step::Attempt {
            state: 10,
            ok: 7,
            nok: 5,
        },
        // A blank line.
        (4, Some(b'\n')) => {
            tokenizer.consume();
            Step::Next(4)
        }
        (4 | 5, Some(_)) => {
            tokenizer.enter("line");
            tokenizer.consume();
            Step::Next(6)
        }
        (6, Some(b'\n')) => {
            tokenizer.exit("line");
            tokenizer.consume();
            Step::Next(4)
        }
        (6, Some(_)) => {
            tokenizer.consume();
            Step::Next(6)
        }
        (6, None) => {
            tokenizer.exit("line");
            tokenizer.exit("fenced");
            Step::Ok
        }
        (3 | 4, None) => {
            if state == 3 {
                tokenizer.exit("fence");
            }
            tokenizer.exit("fenced");
            Step::Ok
        }
        (10, Some(b'%')) => {
            tokenizer.enter("fence");
            tokenizer.consume();
            Step::Next(11)
        }
        (11 | 12, Some(b'%')) => {
            tokenizer.consume();
            Step::Next(state + 1)
        }
        (13, None | Some(b'\n')) => {
            tokenizer.exit("fence");
            Step::Ok
        }
        (7, None | Some(b'\n')) => {
            tokenizer.exit("fenced");
            Step::Ok
        }
        _ => Step::Nok,
    }
}

#[test]
fn flow_construct_takes_lines() {
    let flow = || scripted_flow(b'%', fenced);
    let tokens = |input: &str| {
        find_scripted(&to_mdast(input, &flow()).unwrap()).map(|node| {
            (
                node.value.clone().unwrap_or_default(),
                node.fields["tokens"].clone(),
            )
        })
    };

    assert_eq!(
        tokens("%%%\na\n\nb\n%%%\nc"),
        Some((
            "%%%\na\n\nb\n%%%".into(),
            "fenced,fence,line,line,fence".into()
        )),
        "should take lines up to a closing fence, blank ones too"
    );
    assert_eq!(
        tokens("> %%%\n> a\n>\n> %%%"),
        Some(("%%%\na\n\n%%%".into(), "fenced,fence,line,fence".into())),
        "should leave container prefixes out of lines"
    );
    assert_eq!(
        tokens("%%%\na"),
        Some(("%%%\na".into(), "fenced,fence,line".into())),
        "should end at the end"
    );

    let tree = to_mdast("> %%%\n> a\nb", &flow()).unwrap();
    assert!(
        matches!(
            &tree.children().unwrap()[..],
            [Node::Blockquote(_), Node::Paragraph(_)]
        ),
        "should end before a lazy line, got {:?}",
        tree
    );
    assert_eq!(
        find_scripted(&tree).and_then(|node| node.value.as_deref()),
        Some("%%%\na"),
        "should end the construct before the lazy line"
    );
    assert_eq!(
        tokens("> %%%\n> - a\n> %%%"),
        Some(("%%%\n- a\n%%%".into(), "fenced,fence,line,fence".into())),
        "should not start containers while the construct runs"
    );
    assert!(
        matches!(
            &to_mdast("%%%\n%%%\n> b", &flow())
                .unwrap()
                .children()
                .unwrap()[..],
            [Node::Custom(_), Node::Blockquote(_)]
        ),
        "should allow containers after the construct"
    );

    // `%` and a line, then another line, which a lazy line is not.
    let two_lines: StepFn = |state, tokenizer| match (state, tokenizer.current()) {
        (0, Some(b'%')) => {
            tokenizer.enter("a");
            tokenizer.consume();
            Step::Next(1)
        }
        (1, Some(b'\n')) => {
            tokenizer.consume();
            Step::Next(2)
        }
        (1..=3, Some(byte)) if byte != b'\n' => {
            tokenizer.consume();
            Step::Next(if state == 1 { 1 } else { 3 })
        }
        (3, None | Some(b'\n')) => {
            tokenizer.exit("a");
            Step::Ok
        }
        _ => Step::Nok,
    };
    assert_eq!(
        to_mdast("> %a\nb", &scripted_flow(b'%', two_lines))
            .unwrap_err()
            .rule_id
            .as_str(),
        "flow-construct-late-failure",
        "should error for a failure at a lazy line after the first line"
    );

    let mut found = vec![];
    values(
        &to_mdast("> %%%\n> a\nb\n\n%%%\nc\n%%%\n%d", &flow()).unwrap(),
        &mut found,
    );
    assert_eq!(
        found,
        vec!["%%%\na", "%%%\nc\n%%%"],
        "should start each construct afresh, after one that ended at a lazy line"
    );

    assert_eq!(
        html("%%%\n<a>\n%%%", flow()),
        "%%%\n&lt;a&gt;\n%%%",
        "should write the source of a flow construct as text"
    );
}

/// `@` and a letter on a line of its own, after bytes that `allow` accepts.
struct FlowAfter(fn(Option<u8>) -> bool);

impl Construct for FlowAfter {
    fn markers(&self) -> &[u8] {
        b"@"
    }

    fn previous(&self, previous: Option<u8>) -> bool {
        (self.0)(previous)
    }

    fn step(&self, state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        match (state, tokenizer.current()) {
            (0, Some(b'@')) => {
                tokenizer.enter("mention");
                tokenizer.consume();
                Step::Next(1)
            }
            (1, Some(b'a'..=b'z')) => {
                tokenizer.consume();
                Step::Next(2)
            }
            (2, None | Some(b'\n')) => {
                tokenizer.exit("mention");
                Step::Ok
            }
            _ => Step::Nok,
        }
    }

    fn to_mdast(&self, tokens: Vec<Token>) -> Node {
        mention().to_mdast(tokens)
    }
}

#[test]
fn flow_constructs_see_a_line_ending_before_them() {
    let after = |allow: fn(Option<u8>) -> bool| ParseOptions {
        flow_constructs: vec![Box::new(FlowAfter(allow))],
        ..ParseOptions::default()
    };
    let line_ending = after(|previous| previous == Some(b'\n'));
    let start = after(|previous| previous.is_none());

    for (input, expected) in [
        ("@a", false),
        ("x\n\n@a", true),
        ("x\n\n  @a", true),
        ("> x\n>\n> @a", true),
        ("- x\n\n  @a", true),
    ] {
        assert_eq!(
            has_custom(&to_mdast(input, &line_ending).unwrap()),
            expected,
            "should see a line ending before a line, after indentation and prefixes, in {:?}",
            input
        );
    }

    for (input, expected) in [("@a", true), ("  @a", true), ("x\n\n@a", false)] {
        assert_eq!(
            has_custom(&to_mdast(input, &start).unwrap()),
            expected,
            "should see `None` before the first line in {:?}",
            input
        );
    }
}

#[test]
fn broken_flow_constructs_leave_text() {
    let cases: Vec<(&str, StepFn)> = vec![
        ("`Ok` after consuming", |state, tokenizer| match state {
            0 => {
                tokenizer.enter("a");
                tokenizer.consume();
                Step::Next(1)
            }
            _ => {
                tokenizer.consume();
                tokenizer.exit("a");
                Step::Ok
            }
        }),
        (
            "`Ok` before the end of a line",
            |state, tokenizer| match state {
                0 => {
                    tokenizer.enter("a");
                    tokenizer.consume();
                    Step::Next(1)
                }
                _ => {
                    tokenizer.exit("a");
                    Step::Ok
                }
            },
        ),
        (
            "an exit right after a line ending",
            |state, tokenizer| match state {
                0 | 1 => {
                    if state == 0 {
                        tokenizer.enter("a");
                    }
                    tokenizer.consume();
                    Step::Next(state + 1)
                }
                2 => {
                    tokenizer.consume();
                    tokenizer.exit("a");
                    Step::Next(3)
                }
                _ => Step::Nok,
            },
        ),
    ];

    for (label, step) in cases {
        let tree = to_mdast("%b\nc", &scripted_flow(b'%', step)).unwrap();
        assert!(
            find_scripted(&tree).is_none(),
            "should not match a flow construct that breaks a rule: {}",
            label
        );
    }
}

#[test]
fn flow_constructs_see_their_indent() {
    let two: StepFn = |state, tokenizer| {
        if state == 0 && tokenizer.indent() != 2 {
            Step::Nok
        } else {
            leaf(state, tokenizer)
        }
    };

    for (input, expected) in [("  ::a[b]", true), ("::a[b]", false), (" ::a[b]", false)] {
        assert_eq!(
            find_scripted(&to_mdast(input, &scripted_flow(b':', two)).unwrap()).is_some(),
            expected,
            "should give the columns of indentation in {:?}",
            input
        );
    }
}

#[test]
fn parses_flow_content_across_lines() {
    // `%`, then content up to the end or a lazy line.
    let rest: StepFn = |state, tokenizer| match (state, tokenizer.current()) {
        (0, Some(b'%')) => {
            tokenizer.enter("a");
            tokenizer.consume();
            tokenizer.enter_content("b", ContentType::Text);
            Step::Next(1)
        }
        (1, Some(_)) => {
            tokenizer.consume();
            Step::Next(1)
        }
        (1, None) => {
            tokenizer.exit("b");
            tokenizer.exit("a");
            Step::Ok
        }
        _ => Step::Nok,
    };

    assert_eq!(
        to_mdast("%a\n\nb", &scripted_flow(b'%', rest))
            .unwrap_err()
            .rule_id
            .as_str(),
        "flow-construct-late-failure",
        "should break content with a blank line after the first line"
    );

    for (input, expected) in [
        ("%`a\nb\nc", "`a\nb\nc"),
        ("> %`a\n> b\n> c", "`a\nb\nc"),
        ("%b\n", "b"),
        ("%b\r\n", "b"),
        ("> %a\n> b\n", "a\nb"),
        ("- %a\n", "a"),
    ] {
        let tree = to_mdast(input, &scripted_flow(b'%', rest)).unwrap();
        let node = find_scripted(&tree).expect("expected a match");

        assert_eq!(
            Node::Paragraph(Paragraph {
                children: node.children.clone(),
                position: None
            })
            .to_string(),
            expected,
            "should keep every line of content, and end before a final line ending, in {:?}",
            input
        );
    }

    for input in ["%*a\nb*", "> %*a\n> b*", "> %*a\n> b*\nc"] {
        let tree = to_mdast(input, &scripted_flow(b'%', rest)).unwrap();
        let node = find_scripted(&tree).expect("expected a match");

        assert!(
            matches!(&node.children[..], [Node::Emphasis(emphasis)] if Node::Emphasis(emphasis.clone()).to_string() == "a\nb"),
            "should parse content across lines, without prefixes, in {:?}, got {:?}",
            input,
            node.children
        );
    }
}

/// `:::`, a body parsed as a document, and `:::`, which is tried as an
/// attempt at each line.
fn container(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (0, Some(b':')) => {
            tokenizer.enter("container");
            tokenizer.enter("containerFence");
            tokenizer.consume();
            Step::Next(1)
        }
        (1 | 2, Some(b':')) => {
            tokenizer.consume();
            Step::Next(state + 1)
        }
        (3, Some(b':')) => {
            tokenizer.consume();
            Step::Next(3)
        }
        (3, None | Some(b'\n')) => {
            tokenizer.exit("containerFence");
            Step::Retry(4)
        }
        (4, Some(b'\n')) => {
            tokenizer.consume();
            Step::Next(5)
        }
        (4 | 5, None) => {
            tokenizer.exit("container");
            Step::Ok
        }
        (5, Some(_)) => {
            tokenizer.enter_content("containerContent", ContentType::Document);
            Step::Retry(10)
        }
        (10, Some(_)) => Step::Attempt {
            state: 20,
            ok: 30,
            nok: 11,
        },
        (10 | 11, None) => {
            tokenizer.exit("containerContent");
            tokenizer.exit("container");
            Step::Ok
        }
        (11, Some(b'\n')) => {
            tokenizer.consume();
            Step::Next(10)
        }
        (11, Some(_)) => {
            tokenizer.consume();
            Step::Next(11)
        }
        (20, Some(b':')) => {
            tokenizer.enter("containerFence");
            tokenizer.consume();
            Step::Next(21)
        }
        (21 | 22, Some(b':')) => {
            tokenizer.consume();
            Step::Next(state + 1)
        }
        (23, Some(b':')) => {
            tokenizer.consume();
            Step::Next(23)
        }
        (23, None | Some(b'\n')) => {
            tokenizer.exit("containerFence");
            Step::Ok
        }
        (30, _) => {
            tokenizer.exit("containerContent");
            tokenizer.exit("container");
            Step::Ok
        }
        _ => Step::Nok,
    }
}

fn container_body(input: &str) -> Vec<Node> {
    let tree = to_mdast(input, &scripted_flow(b':', container)).unwrap();
    find_scripted(&tree)
        .expect("expected a match")
        .children
        .clone()
}

#[test]
fn parses_a_body_as_a_document() {
    assert!(
        matches!(
            &container_body(":::\n> *a*\n:::")[..],
            [Node::Blockquote(_)]
        ),
        "should parse containers in the body"
    );

    let body = container_body("> :::\n> - a\n>\n>   b\n> :::");
    assert!(
        matches!(&body[..], [Node::List(list)] if matches!(&list.children[..], [Node::ListItem(item)] if item.spread && item.children.len() == 2)),
        "should parse a spread list item in a body in a block quote, got {:?}",
        body
    );
    assert_eq!(
        body[0].children().unwrap()[0].children().unwrap()[0].position(),
        Some(&Position::new(2, 5, 10, 2, 6, 11)),
        "should start the first line of a body after its prefixes"
    );

    let tree = to_mdast(":::\na\n:::\nb", &scripted_flow(b':', container)).unwrap();
    assert_eq!(
        find_scripted(&tree).and_then(|node| node.fields.get("tokens").cloned()),
        Some("container,containerFence,containerContent".into()),
        "should not give tokens in content, such as the closing fence, to `to_mdast`"
    );
    assert!(
        matches!(tree.children().unwrap().last(), Some(Node::Paragraph(_))),
        "should end at the closing fence"
    );
}

#[test]
fn resolves_definitions_across_bodies() {
    for input in [":::\n[x]: /u\n:::\n\n[x]", "[x]\n\n:::\n[x]: /u\n:::"] {
        let tree = to_mdast(input, &scripted_flow(b':', container)).unwrap();
        assert!(
            format!("{:?}", tree).contains("LinkReference"),
            "should resolve a reference with a definition in a body: {:?}",
            input
        );
    }
}

#[test]
fn ends_a_flow_construct_at_a_lazy_line_or_the_end() {
    let tree = to_mdast("> :::\n> a\nb", &scripted_flow(b':', container)).unwrap();
    assert!(
        matches!(
            &tree.children().unwrap()[..],
            [Node::Blockquote(_), Node::Paragraph(_)]
        ),
        "should end before a lazy line, got {:?}",
        tree
    );

    assert!(
        matches!(&container_body(":::\na\n\n")[..], [Node::Paragraph(_)]),
        "should run to the end without a closing fence"
    );
}

/// `{` and `}` fences around a document whose lines can start with a two
/// space prefix, a token in content.
fn braced(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (0, Some(b'{')) => {
            tokenizer.enter("f");
            tokenizer.enter("open");
            tokenizer.consume();
            tokenizer.exit("open");
            Step::Next(1)
        }
        (1, Some(b'\n')) => {
            tokenizer.consume();
            Step::Next(2)
        }
        (2, _) => {
            tokenizer.enter_content("body", ContentType::Document);
            Step::Retry(3)
        }
        (3, None) => {
            tokenizer.exit("body");
            tokenizer.exit("f");
            Step::Ok
        }
        (3, _) => Step::Attempt {
            state: 10,
            ok: 20,
            nok: 4,
        },
        (4, Some(b' ')) => {
            tokenizer.enter("prefix");
            tokenizer.consume();
            Step::Next(5)
        }
        (4, _) => Step::Retry(6),
        (5, Some(b' ')) => {
            tokenizer.consume();
            tokenizer.exit("prefix");
            Step::Next(6)
        }
        (5, _) => {
            tokenizer.exit("prefix");
            Step::Retry(6)
        }
        (6, Some(b'\n')) => {
            tokenizer.consume();
            Step::Next(3)
        }
        (6, None) => Step::Retry(3),
        (6, Some(_)) => {
            tokenizer.consume();
            Step::Next(6)
        }
        (10, Some(b'}')) => {
            tokenizer.enter("close");
            tokenizer.consume();
            tokenizer.exit("close");
            Step::Next(11)
        }
        (11, None | Some(b'\n')) => Step::Ok,
        (20, _) => {
            tokenizer.exit("body");
            tokenizer.exit("f");
            Step::Ok
        }
        _ => Step::Nok,
    }
}

#[test]
fn leaves_prefixes_of_an_outer_match_out_of_an_inner_one() {
    let tree = to_mdast("{\n  {\n  a\n  }\n}", &scripted_flow(b'{', braced)).unwrap();
    let outer = find_scripted(&tree).expect("expected a match");
    let inner = outer
        .children
        .iter()
        .find_map(find_scripted)
        .expect("expected a nested match");

    assert_eq!(
        (
            inner.fields.get("tokens").map(String::as_str),
            inner.value.as_deref()
        ),
        (Some("f,open,body"), Some("{\n")),
        "should leave the prefix of the outer match out of the inner one"
    );
}

#[test]
fn allows_blank_lines_in_a_body() {
    assert!(
        matches!(
            &container_body(":::\na\n\nb\n:::")[..],
            [Node::Paragraph(_), Node::Paragraph(_)]
        ),
        "should parse blank lines in a document"
    );
}

#[test]
fn parses_an_indented_body_of_lines() {
    let body = container_body("  :::\n  a\n  b\n:::");

    assert!(
        matches!(&body[..], [Node::Paragraph(_)]),
        "should parse the lines of an indented body, got {:?}",
        body
    );
    assert_eq!(body[0].to_string(), "a\nb");
}

/// `!` and a line ending, then lines with a `|` prefix, whose rest is a
/// document; with `TAKE_EOL`, a prefix takes the line ending of a line that
/// has nothing else.
fn bars<const TAKE_EOL: bool>(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (0, Some(b'!')) => {
            tokenizer.enter("bars");
            tokenizer.consume();
            Step::Next(1)
        }
        (1, Some(b'\n')) => {
            tokenizer.consume();
            Step::Next(2)
        }
        (2, Some(b'|')) => {
            if tokenizer.memory()[0] == 0 {
                tokenizer.enter_content("barsBody", ContentType::Document);
                tokenizer.memory()[0] = 1;
            }
            tokenizer.enter("barsPrefix");
            tokenizer.consume();
            Step::Next(3)
        }
        (3, Some(b'\n')) if TAKE_EOL => {
            tokenizer.consume();
            Step::Next(5)
        }
        (5, _) => {
            tokenizer.exit("barsPrefix");
            Step::Retry(2)
        }
        (3, _) => {
            tokenizer.exit("barsPrefix");
            Step::Retry(4)
        }
        (4, Some(b'\n')) => {
            tokenizer.consume();
            Step::Next(2)
        }
        (4, Some(_)) => {
            tokenizer.consume();
            Step::Next(4)
        }
        (2 | 4, None) => {
            tokenizer.exit("barsBody");
            tokenizer.exit("bars");
            Step::Ok
        }
        _ => Step::Nok,
    }
}

#[test]
fn keeps_line_endings_of_a_body_in_the_body() {
    let tree = to_mdast("!\n|a\n|\n|b", &scripted_flow(b'!', bars::<false>)).unwrap();
    assert!(
        matches!(
            &find_scripted(&tree).expect("expected a match").children[..],
            [Node::Paragraph(_), Node::Paragraph(_)]
        ),
        "should parse a line with only a prefix as a blank line"
    );

    assert_eq!(
        to_mdast("!\n|a\n|\n|b", &scripted_flow(b'!', bars::<true>))
            .unwrap_err()
            .rule_id
            .as_str(),
        "flow-construct-late-failure",
        "should break a construct whose token in a body takes a line ending"
    );
}

#[test]
fn writes_bodies_as_html() {
    assert_eq!(
        html("{\n[a]:\n b", scripted_flow(b'{', braced)),
        "{\n",
        "should skip prefixes in definitions in a body"
    );
    assert!(
        html("- :::\n  a\n  :::", scripted_flow(b':', container)).contains("<p>a</p>"),
        "should not make a body in a tight list tight"
    );
}

#[test]
fn ends_a_construct_at_the_end_of_a_body() {
    let parse = ParseOptions {
        flow_constructs: vec![
            Box::new(Scripted {
                marker: b':',
                step: container,
            }),
            Box::new(Scripted {
                marker: b'%',
                step: fenced,
            }),
        ],
        ..ParseOptions::default()
    };
    let tree = to_mdast(":::\n%%%\nx\n:::", &parse).unwrap();
    let body = &find_scripted(&tree).expect("expected a match").children;

    assert_eq!(
        body.iter()
            .find_map(find_scripted)
            .and_then(|node| node.value.as_deref()),
        Some("%%%\nx"),
        "should end before the last line ending of a body, as at the end"
    );
}

#[test]
fn parses_more_in_bodies() {
    assert!(
        matches!(&container_body(":::\r\na\r\n:::")[..], [Node::Paragraph(paragraph)] if paragraph.children[0].to_string() == "a"),
        "should support CR+LF"
    );

    let parse = ParseOptions {
        constructs: markdown::Constructs::gfm(),
        ..scripted_flow(b':', container)
    };
    let tree = to_mdast(":::\n[^a]: b\n:::\n\nc[^a]", &parse).unwrap();
    assert!(
        format!("{:?}", tree).contains("FootnoteReference"),
        "should resolve a footnote with a definition in a body"
    );

    let late: StepFn = |state, tokenizer| match (state, tokenizer.current()) {
        (0, Some(b'%')) => {
            tokenizer.enter("late");
            tokenizer.consume();
            Step::Next(1)
        }
        (1, Some(b'\n')) => {
            tokenizer.consume();
            Step::Next(2)
        }
        (1, Some(_)) => {
            tokenizer.consume();
            Step::Next(1)
        }
        _ => Step::Nok,
    };
    let parse = ParseOptions {
        flow_constructs: vec![
            Box::new(Scripted {
                marker: b':',
                step: container,
            }),
            Box::new(Scripted {
                marker: b'%',
                step: late,
            }),
        ],
        ..ParseOptions::default()
    };
    assert_eq!(
        to_mdast(":::\n%a\nb\n:::", &parse)
            .unwrap_err()
            .rule_id
            .as_str(),
        "flow-construct-late-failure",
        "should error for a late failure in a body"
    );
}

#[test]
fn limits_the_depth_of_bodies() {
    let parse = |levels: usize| {
        let mut input = String::new();
        for level in 0..levels {
            input.push_str(&format!("{}{{\n", "  ".repeat(level)));
        }
        input.push_str(&format!("{}a\n", "  ".repeat(levels)));
        for level in (0..levels).rev() {
            input.push_str(&format!("{}}}\n", "  ".repeat(level)));
        }
        to_mdast(&input, &scripted_flow(b'{', braced))
    };
    assert_eq!(depth(&parse(32).unwrap()), 32, "should nest bodies 32 deep");
    assert_eq!(
        parse(33).unwrap_err().rule_id.as_str(),
        "flow-construct-late-failure",
        "should fail a body deeper than that, which here is past the first line"
    );
}

/// `|` and an optional space or column of a tab as the prefix of the lines
/// of a container, checked from state 10 on later lines.
fn line_block(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (0, Some(b'|')) => {
            tokenizer.enter("lineBlock");
            tokenizer.enter("lineBlockPrefix");
            tokenizer.consume();
            Step::Next(1)
        }
        (1 | 11, Some(b'\t' | b' ')) => {
            tokenizer.consume();
            tokenizer.exit("lineBlockPrefix");
            Step::Next(state + 1)
        }
        (1 | 11, _) => {
            tokenizer.exit("lineBlockPrefix");
            Step::Retry(state + 1)
        }
        (2, _) => {
            tokenizer.enter_content("lineBlockContent", ContentType::Document);
            Step::Ok
        }
        (10, Some(b'|')) => {
            tokenizer.enter("lineBlockPrefix");
            tokenizer.consume();
            Step::Next(11)
        }
        (12, _) => Step::Ok,
        _ => Step::Nok,
    }
}

/// A container at `|`, scripted by a function, with `tokens[0].value` as its
/// value and the names of its tokens as `tokens`.
struct Contained {
    step: StepFn,
    continuation: Option<u16>,
}

impl Construct for Contained {
    fn markers(&self) -> &[u8] {
        b"|"
    }

    fn continuation(&self) -> Option<u16> {
        self.continuation
    }

    fn step(&self, state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        (self.step)(state, tokenizer)
    }

    fn to_mdast(&self, tokens: Vec<Token>) -> Node {
        Node::Custom(Custom {
            name: "container".into(),
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
            children: tokens
                .into_iter()
                .flat_map(|token| token.children)
                .collect(),
            ..Custom::default()
        })
    }
}

fn contained(step: StepFn, continuation: Option<u16>) -> ParseOptions {
    ParseOptions {
        document_constructs: vec![Box::new(Contained { step, continuation })],
        ..ParseOptions::default()
    }
}

fn line_blocks() -> ParseOptions {
    contained(line_block, Some(10))
}

/// Children of the root, with each container as its name and its children.
fn outline(node: &Node) -> String {
    node.children()
        .unwrap()
        .iter()
        .map(|child| match child {
            Node::Custom(_) => format!("container({})", outline(child)),
            Node::Paragraph(_) => "paragraph".into(),
            Node::Heading(_) => "heading".into(),
            Node::List(list) => format!("list{}", list.children.len()),
            Node::Code(_) => "code".into(),
            Node::Blockquote(_) => format!("blockquote({})", outline(child)),
            node => format!("{:?}", node),
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn outline_of(input: &str, parse: &ParseOptions) -> String {
    outline(&to_mdast(input, parse).unwrap())
}

#[test]
fn containers_hold_flow() {
    let parse = line_blocks();

    for (input, expected, message) in [
        ("| a", "container(paragraph)", "should start a container"),
        (
            "| a\n| b",
            "container(paragraph)",
            "should keep one paragraph across prefixes",
        ),
        (
            "| a\n| ===",
            "container(heading)",
            "should find a setext heading across prefixes",
        ),
        (
            "| - a\n| - b",
            "container(list2)",
            "should keep one list across prefixes",
        ),
        (
            "| a\nb",
            "container(paragraph)",
            "should continue a paragraph on a lazy line",
        ),
        ("| | a", "container(container(paragraph))", "should nest"),
        (
            "| a\n\nb",
            "container(paragraph),paragraph",
            "should end at a line without its prefix",
        ),
        (
            "|\ta",
            "container(paragraph)",
            "should take a column of a tab",
        ),
        (
            "|\t\ta",
            "container(code)",
            "should leave the rest of a tab, like `>`",
        ),
        (
            "> | a\n> | b",
            "blockquote(container(paragraph))",
            "should be in block quotes",
        ),
        (
            "| > a\n| > b",
            "container(blockquote(paragraph))",
            "should hold block quotes",
        ),
        ("- | a\n  | b", "list1", "should be in list items"),
        (
            "   | a",
            "container(paragraph)",
            "should start after indentation",
        ),
        ("    | a", "code", "should not start in indented code"),
        (
            "a\n| b",
            "paragraph,container(paragraph)",
            "should interrupt a paragraph",
        ),
    ] {
        assert_eq!(
            outline_of(input, &parse),
            expected,
            "{}: {:?}",
            message,
            input
        );
    }

    let tree = to_mdast("| a\n| b", &parse).unwrap();
    let node = custom(&tree.children().unwrap()[0]);
    assert_eq!(
        (
            node.value.as_deref(),
            node.fields.get("tokens").map(String::as_str)
        ),
        (
            Some("| "),
            Some("lineBlock,lineBlockPrefix,lineBlockContent")
        ),
        "should give the tokens of its first line, not the prefixes of later ones"
    );
    assert_eq!(
        node.position
            .as_ref()
            .map(|position| (position.start.offset, position.end.offset)),
        Some((0, 7)),
        "should span its lines"
    );
}

#[test]
fn separates_lists_across_an_inner_container() {
    assert_eq!(
        outline_of("| - a\n| | x\n| - b", &line_blocks()),
        "container(list1,container(paragraph),list1)",
        "should not skip an inner container when looking past prefixes"
    );
    assert_eq!(
        outline_of("> - a\n> > x\n> - b", &ParseOptions::default()),
        "blockquote(list1,blockquote(paragraph),list1)",
        "should match block quotes"
    );
}

#[test]
fn keeps_list_items_tight_in_containers() {
    assert!(
        html("| - a\n|", line_blocks()).contains("<li>a</li>"),
        "should see past a prefix before the end of a list item"
    );
}

#[test]
fn writes_containers_as_html() {
    assert_eq!(
        html("| *a*\n| b", line_blocks()),
        "| \n<p><em>a</em>\nb</p>",
        "should write the source of its first line and render its content"
    );
}

#[test]
fn leaves_container_prefixes_out_of_values() {
    let parse = ParseOptions {
        text_constructs: vec![Box::new(Scripted {
            marker: b'{',
            step: braces,
        })],
        ..line_blocks()
    };
    let tree = to_mdast("| {{a\n| b}}", &parse).unwrap();

    assert_eq!(
        find_scripted(&tree).and_then(|node| node.attributes.get("bracesData").cloned()),
        Some("a\nb".into()),
        "should leave out the prefix of a container, like `> `"
    );
}

#[test]
fn ends_a_container_without_a_continuation() {
    let parse = contained(line_block, None);
    assert_eq!(
        outline_of("| a\n| b", &parse),
        "container(paragraph),container(paragraph)",
        "should end at the next line"
    );
    assert_eq!(
        outline_of("| a\nb", &parse),
        "container(paragraph)",
        "should still continue a paragraph on a lazy line"
    );
}

#[test]
fn tries_the_next_container_after_no_match() {
    let parse = ParseOptions {
        document_constructs: vec![
            Box::new(Contained {
                step: |_, _| Step::Nok,
                continuation: None,
            }),
            Box::new(Contained {
                step: line_block,
                continuation: Some(10),
            }),
        ],
        ..ParseOptions::default()
    };
    assert_eq!(outline_of("| a\n| b", &parse), "container(paragraph)");
}

#[test]
fn runs_before_builtin_containers() {
    /// `>` as the prefix of the lines of a container.
    struct Quote;

    impl Construct for Quote {
        fn markers(&self) -> &[u8] {
            b">"
        }

        fn continuation(&self) -> Option<u16> {
            Some(10)
        }

        fn step(&self, state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
            match (state, tokenizer.current()) {
                (0, Some(b'>')) => {
                    tokenizer.enter("quote");
                    tokenizer.consume();
                    Step::Next(1)
                }
                (1, _) => {
                    tokenizer.enter_content("quoteContent", ContentType::Document);
                    Step::Ok
                }
                (10, Some(b'>')) => {
                    tokenizer.enter("quotePrefix");
                    tokenizer.consume();
                    tokenizer.exit("quotePrefix");
                    Step::Next(11)
                }
                (11, _) => Step::Ok,
                _ => Step::Nok,
            }
        }

        fn to_mdast(&self, tokens: Vec<Token>) -> Node {
            Contained {
                step: line_block,
                continuation: None,
            }
            .to_mdast(tokens)
        }
    }

    let parse = ParseOptions {
        document_constructs: vec![Box::new(Quote)],
        ..ParseOptions::default()
    };
    assert_eq!(outline_of(">a\n>b", &parse), "container(paragraph)");
}

#[test]
fn parses_flow_like_a_block_quote() {
    let parse = line_blocks();

    for quote in [
        "> # a\n> b\n> c",
        "># a\n>b\n> c",
        "> a\n> - b\n>   c\n>\n> d",
        "> a\nb\n> c",
        "> ```\n> a\n\nb",
        ">     a\n>     b",
        ">\t\ta",
        "> - a\n>\n>   b",
        "> 1. a\n>\n> 2. b",
        "> [a]: b\n>\n> [a]",
        "> a\n> ***\n> b",
        "> <div>\n> a\n\nb",
        ">\n> a\n>",
    ] {
        // Markers at the start of lines.
        let line_block = quote
            .split('\n')
            .map(|line| {
                let markers = line.len() - line.trim_start_matches('>').len();
                format!("{}{}", "|".repeat(markers), &line[markers..])
            })
            .collect::<Vec<_>>()
            .join("\n");
        let quoted = to_mdast(quote, &ParseOptions::default()).unwrap();
        let contained = to_mdast(&line_block, &parse).unwrap();
        let first = |tree: &Node| tree.children().unwrap()[0].children().unwrap().to_vec();
        assert_eq!(
            first(&contained),
            first(&quoted),
            "should parse the content of {:?} like a block quote",
            line_block
        );
    }
}

#[test]
fn broken_containers_leave_text() {
    let cases: [(&str, StepFn, &str); 7] = [
        (
            "bytes in its content token",
            |state, tokenizer| match (state, tokenizer.current()) {
                (2, _) => {
                    tokenizer.enter_content("lineBlockContent", ContentType::Document);
                    tokenizer.consume();
                    Step::Ok
                }
                _ => line_block(state, tokenizer),
            },
            "| a",
        ),
        (
            "its content token in another token",
            |state, tokenizer| match (state, tokenizer.current()) {
                (1, Some(b' ')) => {
                    tokenizer.consume();
                    tokenizer.enter_content("lineBlockContent", ContentType::Document);
                    Step::Ok
                }
                _ => line_block(state, tokenizer),
            },
            "| a",
        ),
        (
            "a line ending",
            |state, tokenizer| match (state, tokenizer.current()) {
                (1, Some(b'\n')) => {
                    tokenizer.consume();
                    tokenizer.exit("lineBlockPrefix");
                    Step::Next(2)
                }
                _ => line_block(state, tokenizer),
            },
            "|\na",
        ),
        (
            "no content token",
            |state, tokenizer| match (state, tokenizer.current()) {
                (2, _) => Step::Ok,
                _ => line_block(state, tokenizer),
            },
            "| a",
        ),
        (
            "a token as its last open token",
            |state, tokenizer| match (state, tokenizer.current()) {
                (2, _) => {
                    tokenizer.enter("lineBlockInner");
                    tokenizer.consume();
                    Step::Ok
                }
                _ => line_block(state, tokenizer),
            },
            "| a",
        ),
        (
            "text content",
            |state, tokenizer| match (state, tokenizer.current()) {
                (2, _) => {
                    tokenizer.enter_content("lineBlockContent", ContentType::Text);
                    Step::Ok
                }
                _ => line_block(state, tokenizer),
            },
            "| a",
        ),
        (
            "a token in its content",
            |state, tokenizer| match (state, tokenizer.current()) {
                (2, _) => {
                    tokenizer.enter_content("lineBlockContent", ContentType::Document);
                    tokenizer.enter("lineBlockInner");
                    Step::Ok
                }
                _ => line_block(state, tokenizer),
            },
            "| a",
        ),
    ];

    for (rule, step, input) in cases {
        assert_eq!(
            outline_of(input, &contained(step, Some(10))),
            outline_of(input, &ParseOptions::default()),
            "should not match a container with {}",
            rule
        );
    }
}

#[test]
fn broken_continuations_end_containers() {
    let cases: [(&str, StepFn, &str); 4] = [
        (
            "an open token",
            |state, tokenizer| match (state, tokenizer.current()) {
                (11, _) => Step::Ok,
                _ => line_block(state, tokenizer),
            },
            "| a\n| b",
        ),
        (
            "a line ending",
            |state, tokenizer| match (state, tokenizer.current()) {
                (10, Some(b'\n')) => {
                    tokenizer.enter("lineBlockPrefix");
                    tokenizer.consume();
                    tokenizer.exit("lineBlockPrefix");
                    Step::Ok
                }
                _ => line_block(state, tokenizer),
            },
            "| a\n\nb",
        ),
        (
            "content",
            |state, tokenizer| match (state, tokenizer.current()) {
                (12, _) => {
                    tokenizer.enter_content("lineBlockContent", ContentType::Document);
                    Step::Ok
                }
                _ => line_block(state, tokenizer),
            },
            "| a\n| b",
        ),
        (
            "a second token",
            |state, tokenizer| match (state, tokenizer.current()) {
                (12, _) => {
                    tokenizer.enter("lineBlockSecond");
                    tokenizer.consume();
                    tokenizer.exit("lineBlockSecond");
                    Step::Ok
                }
                _ => line_block(state, tokenizer),
            },
            "| a\n| b",
        ),
    ];

    for (rule, step, input) in cases {
        assert_eq!(
            outline_of(input, &contained(step, Some(10))),
            outline_of(input, &contained(line_block, None)),
            "should end a container whose continuation has {}",
            rule
        );
    }

    // The line ending check depends on the container continuing otherwise.
    assert_eq!(
        outline_of(
            "| a\n\nb",
            &contained(
                |state, tokenizer| match state {
                    20 => Step::Ok,
                    _ => line_block(state, tokenizer),
                },
                Some(20)
            )
        ),
        "container(paragraph,paragraph)",
        "should continue with a continuation that takes no bytes"
    );
}

#[test]
fn keeps_a_word_of_memory_for_a_container() {
    // Lines of at most 3, from state 20, which also checks that the other
    // words start at zero on each line.
    let parse = contained(
        |state, tokenizer| match (state, tokenizer.current()) {
            (0, _) => {
                tokenizer.memory()[1] = 1;
                line_block(0, tokenizer)
            }
            (20, _) if tokenizer.memory()[0] < 2 && tokenizer.memory()[1..] == [0, 0, 0] => {
                tokenizer.memory()[0] += 1;
                tokenizer.memory()[1] = 1;
                Step::Retry(10)
            }
            (20, _) => Step::Nok,
            _ => line_block(state, tokenizer),
        },
        Some(20),
    );

    assert_eq!(
        outline_of("| a\n| b\n| c\n| d", &parse),
        "container(paragraph),container(paragraph)",
        "should keep word 0 across the lines of a container"
    );
}

#[test]
fn containers_see_their_indent_on_their_first_line() {
    let first = contained(
        |state, tokenizer| match (state, tokenizer.current()) {
            (0, _) if tokenizer.indent() != 2 => Step::Nok,
            _ => line_block(state, tokenizer),
        },
        Some(10),
    );
    assert_eq!(outline_of("  | a", &first), "container(paragraph)");
    assert_eq!(outline_of("| a", &first), "paragraph");

    let later = contained(
        |state, tokenizer| match (state, tokenizer.current()) {
            (10, _) if tokenizer.indent() != 0 => Step::Nok,
            _ => line_block(state, tokenizer),
        },
        Some(10),
    );
    assert_eq!(
        outline_of("  | a\n| b", &later),
        "container(paragraph)",
        "should see `0` on later lines"
    );
}

#[test]
fn spaces_and_tabs_are_never_container_markers() {
    /// A space as the prefix of a container.
    struct Spaced;

    impl Construct for Spaced {
        fn markers(&self) -> &[u8] {
            b" \t"
        }

        fn step(&self, state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
            match (state, tokenizer.current()) {
                (0, Some(b' ' | b'\t')) => {
                    tokenizer.enter("spaced");
                    tokenizer.consume();
                    Step::Next(1)
                }
                (1, _) => {
                    tokenizer.enter_content("spacedContent", ContentType::Document);
                    Step::Ok
                }
                _ => Step::Nok,
            }
        }

        fn to_mdast(&self, tokens: Vec<Token>) -> Node {
            Contained {
                step: line_block,
                continuation: None,
            }
            .to_mdast(tokens)
        }
    }

    let parse = ParseOptions {
        document_constructs: vec![Box::new(Spaced)],
        ..ParseOptions::default()
    };
    assert_eq!(outline_of("    a", &parse), "code");
    assert_eq!(outline_of("\ta", &parse), "code");
}

#[test]
fn limits_steps_of_a_continuation() {
    // Retries at its first byte: with the step that consumes `|`, 254 fit
    // in the 256 steps one byte allows.
    let lines = |step: StepFn| outline_of("| a\n| b", &contained(step, Some(20)));

    assert_eq!(lines(spend_continuation::<254>), "container(paragraph)");
    assert_eq!(
        lines(spend_continuation::<255>),
        "container(paragraph),container(paragraph)",
        "should count steps from the start of the continuation"
    );
}

fn spend_continuation<const N: usize>(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match state {
        20 if tokenizer.memory()[1] < N => {
            tokenizer.memory()[1] += 1;
            Step::Retry(20)
        }
        20 => Step::Retry(10),
        _ => line_block(state, tokenizer),
    }
}

#[test]
fn containers_see_a_line_ending_before_them() {
    /// A line block, after bytes it allows.
    struct ContainerAfter(fn(Option<u8>) -> bool);

    impl Construct for ContainerAfter {
        fn markers(&self) -> &[u8] {
            b"|"
        }

        fn previous(&self, previous: Option<u8>) -> bool {
            (self.0)(previous)
        }

        fn continuation(&self) -> Option<u16> {
            Some(10)
        }

        fn step(&self, state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
            line_block(state, tokenizer)
        }

        fn to_mdast(&self, tokens: Vec<Token>) -> Node {
            Contained {
                step: line_block,
                continuation: None,
            }
            .to_mdast(tokens)
        }
    }

    let after = |allow: fn(Option<u8>) -> bool| ParseOptions {
        document_constructs: vec![Box::new(ContainerAfter(allow))],
        ..ParseOptions::default()
    };
    let line_ending = after(|previous| previous == Some(b'\n'));
    let start = after(|previous| previous.is_none());

    for (input, expected) in [
        ("| a", false),
        ("x\n\n| a", true),
        ("x\n\n  | a", true),
        ("> x\n> | a", true),
        ("- x\n- | a", true),
    ] {
        assert_eq!(
            has_custom(&to_mdast(input, &line_ending).unwrap()),
            expected,
            "should see a line ending before a line, after indentation and prefixes, in {:?}",
            input
        );
    }

    for (input, expected) in [
        ("| a", true),
        ("  | a", true),
        ("> | a", true),
        ("x\n\n| a", false),
    ] {
        assert_eq!(
            has_custom(&to_mdast(input, &start).unwrap()),
            expected,
            "should see `None` before the first line in {:?}",
            input
        );
    }
}

#[test]
fn nests_containers_in_the_deepest_bodies() {
    let parse = ParseOptions {
        flow_constructs: vec![Box::new(Scripted {
            marker: b'{',
            step: braced,
        })],
        ..line_blocks()
    };
    let mut input = String::new();
    for level in 0..32 {
        input.push_str(&format!("{}{{\n", "  ".repeat(level)));
    }
    input.push_str(&format!("{}| a\n", "  ".repeat(32)));
    for level in (0..32).rev() {
        input.push_str(&format!("{}}}\n", "  ".repeat(level)));
    }
    let tree = to_mdast(&input, &parse).unwrap();
    let mut node = &tree;
    while let Some(child) = node.children().and_then(|children| children.first()) {
        node = child;
        if matches!(node, Node::Custom(custom) if custom.name == "container") {
            break;
        }
    }
    assert!(
        matches!(node, Node::Custom(custom) if custom.name == "container"),
        "should not count containers toward the depth of content"
    );
}

/// Turn containers into block quotes, to compare trees.
fn as_block_quotes(node: &mut Node) {
    if let Node::Custom(custom) = node {
        *node = Node::Blockquote(Blockquote {
            children: std::mem::take(&mut custom.children),
            position: custom.position.take(),
        });
    }
    if let Some(children) = node.children_mut() {
        children.iter_mut().for_each(as_block_quotes);
    }
}

#[test]
fn nests_flow_like_a_block_quote() {
    let parse = line_blocks();

    for quote in [
        "* >\n* a",
        "- >\n\n- a",
        "1. >\n2. a",
        "- > a\n  > b\n- c",
        "- a\n\n  > b\n- c",
        "> - >\n>   a",
        "> > a\n> b",
        "- > - a\n  >\n  > - b",
        "* > a\n  >\n* b",
        "- >\n  >\n- b",
        ">>- a\n>>\n>>-",
        "* > > a\n  > >\n* b",
    ] {
        let quoted = to_mdast(quote, &ParseOptions::default()).unwrap();
        let mut contained = to_mdast(&quote.replace('>', "|"), &parse).unwrap();
        as_block_quotes(&mut contained);
        assert_eq!(
            contained, quoted,
            "should parse {:?} like a block quote",
            quote
        );
    }
}

#[test]
fn keeps_the_budget_of_a_continuation_after_an_outer_one() {
    /// `&` as the prefix of the lines of a container, whose continuation
    /// first tries a token to the end of the line and fails it.
    struct Looking;

    impl Construct for Looking {
        fn markers(&self) -> &[u8] {
            b"&"
        }

        fn continuation(&self) -> Option<u16> {
            Some(20)
        }

        fn step(&self, state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
            match (state, tokenizer.current()) {
                (0 | 10, Some(b'&')) => {
                    if state == 0 {
                        tokenizer.enter("lineBlock");
                    }
                    tokenizer.enter("lineBlockPrefix");
                    tokenizer.consume();
                    Step::Next(state + 1)
                }
                (20, _) => Step::Attempt {
                    state: 30,
                    ok: 10,
                    nok: 10,
                },
                (30, _) => {
                    tokenizer.enter("look");
                    Step::Retry(31)
                }
                (31, Some(byte)) if byte != b'\n' => {
                    tokenizer.consume();
                    Step::Next(31)
                }
                (0 | 10 | 31, _) => Step::Nok,
                _ => line_block(state, tokenizer),
            }
        }

        fn to_mdast(&self, tokens: Vec<Token>) -> Node {
            Contained {
                step: line_block,
                continuation: None,
            }
            .to_mdast(tokens)
        }
    }

    let nested = |step: StepFn| ParseOptions {
        document_constructs: vec![
            Box::new(Looking),
            Box::new(Contained {
                step,
                continuation: Some(20),
            }),
        ],
        ..ParseOptions::default()
    };
    let input = "& | a\n& | b";

    assert_eq!(
        outline_of(input, &nested(spend_continuation::<254>)),
        "container(container(paragraph))"
    );
    assert_eq!(
        outline_of(input, &nested(spend_continuation::<255>)),
        "container(container(paragraph),container(paragraph))",
        "should not give a continuation the bytes an earlier one reached"
    );
}

/// `=` delimiter runs of the given sizes, such as `==a==`, as `mark` nodes
/// with the names of their tokens as `tokens`.
struct Mark {
    markers: &'static [u8],
    sizes: &'static [usize],
    previous: fn(Option<u8>) -> bool,
}

impl Construct for Mark {
    fn markers(&self) -> &[u8] {
        self.markers
    }

    fn previous(&self, previous: Option<u8>) -> bool {
        (self.previous)(previous)
    }

    fn attention_sizes(&self) -> &[usize] {
        self.sizes
    }

    fn step(&self, _: u16, _: &mut ConstructTokenizer) -> Step {
        unreachable!("expected a delimiter run not to step")
    }

    fn to_mdast(&self, tokens: Vec<Token>) -> Node {
        Node::Custom(Custom {
            name: "mark".into(),
            value: Some(tokens[1].value.clone().into_owned()),
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
            children: tokens
                .into_iter()
                .flat_map(|token| token.children)
                .collect(),
            ..Custom::default()
        })
    }
}

fn marks(sizes: &'static [usize]) -> ParseOptions {
    ParseOptions {
        text_constructs: vec![Box::new(Mark {
            markers: b"=",
            sizes,
            previous: |_| true,
        })],
        ..ParseOptions::default()
    }
}

/// Minimal HTML of paragraphs, emphasis, marks, and text.
fn render_marks(node: &Node) -> String {
    let inner = |children: &[Node]| children.iter().map(render_marks).collect::<String>();
    match node {
        Node::Root(root) => inner(&root.children),
        Node::Blockquote(quote) => inner(&quote.children),
        Node::Paragraph(paragraph) => format!("<p>{}</p>", inner(&paragraph.children)),
        Node::Emphasis(emphasis) => format!("<em>{}</em>", inner(&emphasis.children)),
        Node::Delete(delete) => format!("<del>{}</del>", inner(&delete.children)),
        Node::Custom(custom) => format!("<mark>{}</mark>", inner(&custom.children)),
        node => node.to_string(),
    }
}

fn marked(input: &str, parse: &ParseOptions) -> String {
    render_marks(&to_mdast(input, parse).unwrap())
}

#[test]
fn pairs_delimiter_runs() {
    let parse = marks(&[2]);

    for (input, expected, message) in [
        ("==a==", "<p><mark>a</mark></p>", "should pair runs"),
        (
            "==a *b*==",
            "<p><mark>a <em>b</em></mark></p>",
            "should parse text in them",
        ),
        (
            "*==a==*",
            "<p><em><mark>a</mark></em></p>",
            "should allow emphasis around them",
        ),
        (
            "==*a*==",
            "<p><mark><em>a</em></mark></p>",
            "should allow emphasis in them",
        ),
        (
            "==a *b== c*",
            "<p><mark>a *b</mark> c*</p>",
            "should not misnest",
        ),
        (
            "==a ==b== c==",
            "<p><mark>a <mark>b</mark> c</mark></p>",
            "should nest",
        ),
        (
            "===a===",
            "<p>===a===</p>",
            "should not pair sizes it does not allow",
        ),
        (
            "=a=",
            "<p>=a=</p>",
            "should not pair a size it does not allow",
        ),
        (
            "==a===",
            "<p>==a===</p>",
            "should not pair runs of other sizes",
        ),
        (
            "===a==",
            "<p>===a==</p>",
            "should not pair a shorter closing run",
        ),
        (
            "a*==b==*c",
            "<p>a<em><mark>b</mark></em>c</p>",
            "should count as markers around emphasis",
        ),
        (
            "a==*b*==c",
            "<p>a==<em>b</em>==c</p>",
            "should not see markers around itself, like `~`",
        ),
        (
            "== a ==",
            "<p>== a ==</p>",
            "should not open before or close after whitespace",
        ),
        (
            "a==b==c",
            "<p>a<mark>b</mark>c</p>",
            "should pair inside words, like `~`",
        ),
        (
            "> ==a\n> b==",
            "<p><mark>a\nb</mark></p>",
            "should pair across lines",
        ),
    ] {
        assert_eq!(marked(input, &parse), expected, "{}: {:?}", message, input);
    }

    assert_eq!(
        marked("=a= ==b== ===c===", &marks(&[1, 3])),
        "<p><mark>a</mark> ==b== <mark>c</mark></p>",
        "should pair each size it allows"
    );
}

#[test]
fn gives_delimiter_runs_their_tokens() {
    let tree = to_mdast("x ==*a*== y", &marks(&[2])).unwrap();
    let node = custom(&phrasing(tree)[1]).clone();

    assert_eq!(
        (
            node.value.as_deref(),
            node.fields.get("tokens").map(String::as_str)
        ),
        (
            Some("=="),
            Some("attention,attentionSequence,attentionText,attentionSequence")
        ),
        "should give a run, its sequences, and its text"
    );
    assert_eq!(
        node.position
            .as_ref()
            .map(|position| (position.start.offset, position.end.offset)),
        Some((2, 9)),
        "should span the run"
    );
    assert_eq!(
        html("x ==*a*== y", marks(&[2])),
        "<p>x ==<em>a</em>== y</p>",
        "should write the sequences as text in HTML"
    );
}

#[test]
fn checks_the_byte_before_a_delimiter_run() {
    let parse = ParseOptions {
        text_constructs: vec![Box::new(Mark {
            markers: b"=",
            sizes: &[2],
            previous: |previous| previous != Some(b'a'),
        })],
        ..ParseOptions::default()
    };

    assert_eq!(marked("b==c== d", &parse), "<p>b<mark>c</mark> d</p>");
    assert_eq!(marked("a==c== d", &parse), "<p>a==c== d</p>");
}

#[test]
fn delimiter_runs_come_before_builtin_markers() {
    let parse = ParseOptions {
        constructs: markdown::Constructs::gfm(),
        text_constructs: vec![Box::new(Mark {
            markers: b"~",
            sizes: &[3],
            previous: |_| true,
        })],
        ..ParseOptions::gfm()
    };

    assert_eq!(
        marked("x ~~~a~~~ ~~b~~", &parse),
        "<p>x <mark>a</mark> ~~b~~</p>",
        "should pair by the sizes of the construct, not strikethrough"
    );
    assert_eq!(
        marked("x ~~~a~~~ ~~b~~", &ParseOptions::gfm()),
        "<p>x ~~~a~~~ <del>b</del></p>",
        "should otherwise be strikethrough"
    );
}

#[test]
fn leaves_attention_to_constructs_that_are_not_runs() {
    let parse = ParseOptions {
        text_constructs: vec![
            Box::new(Scripted {
                marker: b'*',
                step: |_, _| Step::Nok,
            }),
            Box::new(Mark {
                markers: b"=",
                sizes: &[2],
                previous: |_| true,
            }),
        ],
        ..ParseOptions::default()
    };

    assert_eq!(
        marked("*a* ==b==", &parse),
        "<p><em>a</em> <mark>b</mark></p>"
    );
}

/// A run construct with `markers`, `sizes`, and a `previous` check.
fn run(
    markers: &'static [u8],
    sizes: &'static [usize],
    previous: fn(Option<u8>) -> bool,
) -> Box<dyn Construct> {
    Box::new(Mark {
        markers,
        sizes,
        previous,
    })
}

#[test]
fn delimiter_runs_are_ascii() {
    for markers in [b"\xC2", b"\xA7"] {
        let parse = ParseOptions {
            text_constructs: vec![run(markers, &[1], |_| true)],
            ..ParseOptions::default()
        };
        assert_eq!(
            marked("\u{a7}a\u{a7}", &parse),
            "<p>\u{a7}a\u{a7}</p>",
            "should not start a run inside a character"
        );
    }
}

#[test]
fn pairs_runs_of_the_same_construct() {
    let gfm = ParseOptions {
        text_constructs: vec![run(b"~", &[2], |previous| previous != Some(b'a'))],
        ..ParseOptions::gfm()
    };
    assert_eq!(
        marked("a~~ba~~ c", &gfm),
        "<p>a<del>ba</del> c</p>",
        "should leave what the construct does not take to the built-ins"
    );
    assert_eq!(
        marked("a~~b~~ c", &gfm),
        "<p>a~~b~~ c</p>",
        "should not pair a run with a built-in sequence"
    );
    assert_eq!(marked("b~~c~~ d", &gfm), "<p>b<mark>c</mark> d</p>");

    let shared = ParseOptions {
        text_constructs: vec![
            run(b"=", &[2], |previous| previous != Some(b'a')),
            run(b"=", &[1], |_| true),
        ],
        ..ParseOptions::default()
    };
    assert_eq!(
        marked("a=ba= c ==d==", &shared),
        "<p>a<mark>ba</mark> c <mark>d</mark></p>",
        "should pair runs of each construct at a marker"
    );
}

#[test]
fn delimiter_runs_flank_like_strikethrough() {
    let parse = |markers: &'static [u8]| ParseOptions {
        text_constructs: vec![run(markers, &[1], |_| true)],
        ..ParseOptions::default()
    };
    assert_eq!(
        marked("a*_b_*c", &parse(b"*")),
        "<p>a*<em>b</em>*c</p>",
        "should not see markers around a run at `*`"
    );
    assert_eq!(
        marked("a_b_c", &parse(b"_")),
        "<p>a<mark>b</mark>c</p>",
        "should pair inside words at `_`"
    );
    assert_eq!(
        marked("a *\nb*", &parse(b"=\n")),
        "<p>a *\nb*</p>",
        "should not take a line ending for a marker"
    );
}
