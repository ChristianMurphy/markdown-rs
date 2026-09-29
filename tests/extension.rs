use markdown::{
    extension::{Construct, ConstructTokenizer, ContentType, Step, Token},
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
