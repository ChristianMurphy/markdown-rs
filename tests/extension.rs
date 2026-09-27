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
        to_html_with_options("x @< y", &options).unwrap(),
        "<p>x @&lt; y</p>",
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
fn braces(state: u16, t: &mut ConstructTokenizer) -> Step {
    match (state, t.current()) {
        (0, Some(b'{')) => {
            t.enter("braces");
            t.consume();
            Step::Next(1)
        }
        (1, Some(b'{')) => {
            t.consume();
            t.enter("bracesData");
            Step::Next(2)
        }
        (2, Some(b'}')) => {
            t.exit("bracesData");
            t.consume();
            Step::Next(3)
        }
        (2, Some(_)) => {
            t.consume();
            Step::Next(2)
        }
        (3, Some(b'}')) => {
            t.consume();
            t.exit("braces");
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
        to_html_with_options("> {{a\n> b}}", &options).unwrap(),
        "<blockquote>\n<p>{{a\nb}}</p>\n</blockquote>"
    );
}

#[test]
fn broken_constructs_leave_text() {
    let cases: Vec<(&str, StepFn)> = vec![
        ("`Ok` without consuming", |_, _| Step::Ok),
        ("`Next` without consuming", |_, t| {
            t.enter("a");
            Step::Next(0)
        }),
        ("`Retry` forever", |_, _| Step::Retry(0)),
        ("consuming outside a token", |_, t| {
            t.consume();
            Step::Ok
        }),
        ("closing a token that is not open", |_, t| {
            t.enter("a");
            t.consume();
            t.exit("b");
            Step::Ok
        }),
        ("`Ok` with an open token", |_, t| {
            t.enter("a");
            t.consume();
            Step::Ok
        }),
        ("two outermost tokens", |state, t| match state {
            0 => {
                t.enter("a");
                t.consume();
                t.exit("a");
                t.enter("b");
                Step::Next(1)
            }
            _ => {
                t.consume();
                t.exit("b");
                Step::Ok
            }
        }),
        ("an empty token", |_, t| {
            t.enter("a");
            t.exit("a");
            t.enter("b");
            t.consume();
            t.exit("b");
            Step::Ok
        }),
        (
            "closing, in an attempt, a token opened before it",
            |state, t| match state {
                0 => {
                    t.enter("outer");
                    t.enter("inner");
                    t.consume();
                    t.exit("inner");
                    Step::Next(1)
                }
                1 => Step::Attempt {
                    state: 10,
                    ok: 2,
                    nok: 2,
                },
                10 => {
                    t.exit("outer");
                    Step::Nok
                }
                _ => Step::Ok,
            },
        ),
        (
            "a wrong exit in an attempt, in content",
            |state, t| match state {
                0 => {
                    t.enter("a");
                    t.consume();
                    t.enter_content("b", ContentType::Text);
                    Step::Next(1)
                }
                1 => {
                    t.consume();
                    Step::Next(2)
                }
                2 => Step::Attempt {
                    state: 10,
                    ok: 3,
                    nok: 4,
                },
                3 => {
                    t.exit("b");
                    t.exit("a");
                    Step::Ok
                }
                10 => {
                    t.exit("nope");
                    Step::Ok
                }
                _ => Step::Nok,
            },
        ),
        (
            "an attempt that succeeds without progress, forever",
            |state, t| match state {
                0 => {
                    t.enter("a");
                    t.consume();
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
            |state, t| match state {
                0 => {
                    t.enter("a");
                    t.consume();
                    Step::Next(1)
                }
                1 => Step::Attempt {
                    state: 10,
                    ok: 2,
                    nok: 1,
                },
                10 => {
                    t.enter("b");
                    t.consume();
                    t.exit("b");
                    Step::Next(11)
                }
                _ => Step::Nok,
            },
        ),
        (
            "a token in text content after content on its line",
            |state, t| match state {
                0 => {
                    t.enter("a");
                    t.enter_content("b", ContentType::Text);
                    t.consume();
                    Step::Next(1)
                }
                1 => {
                    t.enter("mid");
                    t.consume();
                    t.exit("mid");
                    t.exit("b");
                    t.exit("a");
                    Step::Ok
                }
                _ => Step::Nok,
            },
        ),
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
fn token_inside_a_character_leaves_text() {
    let options = Options {
        parse: ParseOptions {
            text_constructs: vec![Box::new(Scripted {
                marker: b'{',
                step: |state, t| match state {
                    0 => {
                        t.enter("a");
                        t.consume();
                        Step::Next(1)
                    }
                    _ => {
                        // Consume one byte of the two in `é`.
                        t.consume();
                        t.exit("a");
                        Step::Ok
                    }
                },
            })],
            ..ParseOptions::default()
        },
        ..Options::default()
    };

    assert_eq!(to_html_with_options("{é", &options).unwrap(), "<p>{é</p>");
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
fn percent_to_line_end(state: u16, t: &mut ConstructTokenizer) -> Step {
    match (state, t.current()) {
        (0, Some(b'%')) => {
            t.enter("percent");
            t.consume();
            Step::Next(1)
        }
        (1, Some(b'\n') | None) => {
            t.exit("percent");
            Step::Ok
        }
        (1, Some(_)) => {
            t.consume();
            Step::Next(1)
        }
        _ => Step::Nok,
    }
}

/// Like `percent_to_line_end`, but wrongly consumes the line ending before
/// ending.
fn percent_through_line_end(state: u16, t: &mut ConstructTokenizer) -> Step {
    match (state, t.current()) {
        (1, Some(b'\n')) => {
            t.consume();
            t.exit("percent");
            Step::Ok
        }
        _ => percent_to_line_end(state, t),
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
    fn line_after_line_ending(state: u16, t: &mut ConstructTokenizer) -> Step {
        match (state, t.current()) {
            (2, Some(b'\n')) => {
                t.consume();
                t.enter("line");
                Step::Next(5)
            }
            (5, Some(b'}')) => {
                t.exit("line");
                Step::Retry(2)
            }
            (5, Some(_)) => {
                t.consume();
                Step::Next(5)
            }
            _ => braces(state, t),
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
    fn line_ending_then_ok(state: u16, t: &mut ConstructTokenizer) -> Step {
        match (state, t.current()) {
            (1, Some(b'\n')) => {
                t.consume();
                t.exit("percent");
                Step::Next(2)
            }
            (2, _) => Step::Ok,
            _ => percent_to_line_end(state, t),
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
    fn to_end(state: u16, t: &mut ConstructTokenizer) -> Step {
        match (state, t.current()) {
            (0, _) => {
                t.enter("a");
                t.consume();
                Step::Next(1)
            }
            (_, Some(_)) => {
                t.consume();
                Step::Next(1)
            }
            (_, None) => {
                t.consume();
                t.exit("a");
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

/// `{{`, content parsed as text, `}}`.
fn braces_content(state: u16, t: &mut ConstructTokenizer) -> Step {
    match (state, t.current()) {
        (0, Some(b'{')) => {
            t.enter("braces");
            t.consume();
            Step::Next(1)
        }
        (1, Some(b'{')) => {
            t.consume();
            t.enter_content("bracesContent", ContentType::Text);
            Step::Next(2)
        }
        (2, Some(b'}')) => {
            t.exit("bracesContent");
            t.consume();
            Step::Next(3)
        }
        (2, Some(_)) => {
            t.consume();
            Step::Next(2)
        }
        (3, Some(b'}')) => {
            t.consume();
            t.exit("braces");
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
    let empty: StepFn = |state, t| match state {
        0 => {
            t.enter("a");
            t.consume();
            t.enter_content("b", ContentType::Text);
            Step::Next(1)
        }
        _ => {
            t.exit("b");
            t.exit("a");
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

/// `::name[label]`, with the label parsed as text.
fn leaf(state: u16, t: &mut ConstructTokenizer) -> Step {
    match (state, t.current()) {
        (0 | 1, Some(b':')) => {
            if state == 0 {
                t.enter("leaf");
            }
            t.consume();
            Step::Next(state + 1)
        }
        (2 | 3, Some(b'a'..=b'z')) => {
            if state == 2 {
                t.enter("leafName");
            }
            t.consume();
            Step::Next(3)
        }
        (3, Some(b'[')) => {
            t.exit("leafName");
            t.consume();
            Step::Next(4)
        }
        (4, Some(b']')) => {
            t.consume();
            Step::Next(6)
        }
        (4, Some(_)) => {
            t.enter_content("leafLabel", ContentType::Text);
            Step::Retry(5)
        }
        (5, Some(b']')) => {
            t.exit("leafLabel");
            t.consume();
            Step::Next(6)
        }
        (5, Some(byte)) if byte != b'\n' => {
            t.consume();
            Step::Next(5)
        }
        (6, None | Some(b'\n')) => {
            t.exit("leaf");
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
}

/// `:::`, a body parsed as a document, and `:::`, which is tried as an
/// attempt at each line.
fn container(state: u16, t: &mut ConstructTokenizer) -> Step {
    match (state, t.current()) {
        (0, Some(b':')) => {
            t.enter("container");
            t.enter("containerFence");
            t.consume();
            Step::Next(1)
        }
        (1 | 2, Some(b':')) => {
            t.consume();
            Step::Next(state + 1)
        }
        (3, Some(b':')) => {
            t.consume();
            Step::Next(3)
        }
        (3, None | Some(b'\n')) => {
            t.exit("containerFence");
            Step::Retry(4)
        }
        (4, Some(b'\n')) => {
            t.consume();
            Step::Next(5)
        }
        (4 | 5, None) => {
            t.exit("container");
            Step::Ok
        }
        (5, Some(_)) => {
            t.enter_content("containerContent", ContentType::Document);
            Step::Retry(10)
        }
        (10, Some(_)) => Step::Attempt {
            state: 20,
            ok: 30,
            nok: 11,
        },
        (10 | 11, None) => {
            t.exit("containerContent");
            t.exit("container");
            Step::Ok
        }
        (11, Some(b'\n')) => {
            t.consume();
            Step::Next(10)
        }
        (11, Some(_)) => {
            t.consume();
            Step::Next(11)
        }
        (20, Some(b':')) => {
            t.enter("containerFence");
            t.consume();
            Step::Next(21)
        }
        (21 | 22, Some(b':')) => {
            t.consume();
            Step::Next(state + 1)
        }
        (23, Some(b':')) => {
            t.consume();
            Step::Next(23)
        }
        (23, None | Some(b'\n')) => {
            t.exit("containerFence");
            Step::Ok
        }
        (30, _) => {
            t.exit("containerContent");
            t.exit("container");
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
            tree.to_string().contains('x') && format!("{:?}", tree).contains("LinkReference"),
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

#[test]
fn errors_when_a_flow_construct_fails_after_its_first_line() {
    let late: StepFn = |state, t| match (state, t.current()) {
        (0, Some(b'%')) => {
            t.enter("late");
            t.consume();
            Step::Next(1)
        }
        (1, Some(b'\n')) => {
            t.consume();
            Step::Next(2)
        }
        (1, Some(_)) => {
            t.consume();
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
fn undoes_a_failed_attempt() {
    let optional: StepFn = |state, t| match (state, t.current()) {
        (0, Some(b'{')) => {
            t.enter("a");
            t.consume();
            Step::Next(1)
        }
        (1, _) => Step::Attempt {
            state: 10,
            ok: 2,
            nok: 2,
        },
        (10, Some(b'x')) => {
            t.enter("optional");
            t.consume();
            Step::Next(11)
        }
        (11, Some(b'y')) => {
            t.consume();
            t.exit("optional");
            Step::Ok
        }
        (2, Some(b'}')) => {
            t.consume();
            t.exit("a");
            Step::Ok
        }
        (2, Some(_)) => {
            t.consume();
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
    let step: StepFn = |state, t| match (state, t.current()) {
        (0, Some(b'{')) => {
            t.enter("a");
            t.consume();
            t.enter_content("b", ContentType::Text);
            Step::Next(1)
        }
        (1, Some(b'x')) => {
            t.consume();
            Step::Next(2)
        }
        (2, _) => Step::Attempt {
            state: 10,
            ok: 3,
            nok: 3,
        },
        (10, Some(b'\n')) => {
            t.consume();
            Step::Next(11)
        }
        (3, Some(b'}')) => {
            t.exit("b");
            t.consume();
            t.exit("a");
            Step::Ok
        }
        (3, Some(_)) => {
            t.consume();
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
    let step: StepFn = |state, t| match (state, t.current()) {
        (0, Some(b'{')) => {
            t.enter("a");
            t.consume();
            t.enter_content("b", ContentType::Text);
            Step::Next(1)
        }
        (1, Some(b'x')) => {
            t.consume();
            Step::Next(2)
        }
        (2, _) => Step::Attempt {
            state: 10,
            ok: 3,
            nok: 3,
        },
        // Content up to `y`, which succeeds, or `}`, which fails.
        (10, Some(b'y')) => {
            t.consume();
            Step::Next(11)
        }
        (10, Some(byte)) if byte != b'}' => {
            t.consume();
            Step::Next(10)
        }
        (11, _) => Step::Ok,
        (3, _) => {
            t.exit("b");
            Step::Retry(4)
        }
        (4, Some(b'}')) => {
            t.consume();
            t.exit("a");
            Step::Ok
        }
        (4, Some(_)) => {
            t.consume();
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
fn attempts_in_flow_stay_on_one_line() {
    let across: StepFn = |state, t| match (state, t.current()) {
        (0, Some(b'%')) => {
            t.enter("f");
            t.consume();
            Step::Next(1)
        }
        (1, _) => Step::Attempt {
            state: 10,
            ok: 2,
            nok: 2,
        },
        (10, Some(b'\n')) => {
            t.enter("x");
            t.consume();
            Step::Next(11)
        }
        (2, None | Some(b'\n')) => {
            t.exit("f");
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
}

/// `{` and `}` fences around a document whose lines can start with a two
/// space prefix, a token in content.
fn fenced(state: u16, t: &mut ConstructTokenizer) -> Step {
    match (state, t.current()) {
        (0, Some(b'{')) => {
            t.enter("f");
            t.enter("open");
            t.consume();
            t.exit("open");
            Step::Next(1)
        }
        (1, Some(b'\n')) => {
            t.consume();
            Step::Next(2)
        }
        (2, _) => {
            t.enter_content("body", ContentType::Document);
            Step::Retry(3)
        }
        (3, None) => {
            t.exit("body");
            t.exit("f");
            Step::Ok
        }
        (3, _) => Step::Attempt {
            state: 10,
            ok: 20,
            nok: 4,
        },
        (4, Some(b' ')) => {
            t.enter("prefix");
            t.consume();
            Step::Next(5)
        }
        (4, _) => Step::Retry(6),
        (5, Some(b' ')) => {
            t.consume();
            t.exit("prefix");
            Step::Next(6)
        }
        (5, _) => {
            t.exit("prefix");
            Step::Retry(6)
        }
        (6, Some(b'\n')) => {
            t.consume();
            Step::Next(3)
        }
        (6, None) => Step::Retry(3),
        (6, Some(_)) => {
            t.consume();
            Step::Next(6)
        }
        (10, Some(b'}')) => {
            t.enter("close");
            t.consume();
            t.exit("close");
            Step::Next(11)
        }
        (11, None | Some(b'\n')) => Step::Ok,
        (20, _) => {
            t.exit("body");
            t.exit("f");
            Step::Ok
        }
        _ => Step::Nok,
    }
}

#[test]
fn leaves_prefixes_of_an_outer_match_out_of_an_inner_one() {
    let tree = to_mdast("{\n  {\n  a\n  }\n}", &scripted_flow(b'{', fenced)).unwrap();
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
fn keeps_list_items_tight_in_plugin_containers() {
    let parse = ParseOptions {
        document_constructs: vec![Box::new(LineBlock)],
        ..ParseOptions::default()
    };

    assert!(
        html("| - a\n|", parse).contains("<li>a</li>"),
        "should see past a plugin prefix before the end of a list item"
    );
}

/// `|` line blocks: a container whose later lines start with `|`.
struct LineBlock;

impl Construct for LineBlock {
    fn markers(&self) -> &[u8] {
        b"|"
    }

    fn continuation(&self) -> u16 {
        10
    }

    fn step(&self, state: u16, t: &mut ConstructTokenizer) -> Step {
        match (state, t.current()) {
            (0, Some(b'|')) => {
                t.enter("lineBlock");
                t.enter("lineBlockPrefix");
                t.consume();
                Step::Next(1)
            }
            (1 | 11, Some(b' ')) => {
                t.consume();
                t.exit("lineBlockPrefix");
                Step::Next(state + 1)
            }
            (1 | 11, _) => {
                t.exit("lineBlockPrefix");
                Step::Retry(state + 1)
            }
            (2, _) => {
                t.enter_content("lineBlockContent", ContentType::Document);
                Step::Ok
            }
            (10, Some(b'|')) => {
                t.enter("lineBlockPrefix");
                t.consume();
                Step::Next(11)
            }
            (12, _) => Step::Ok,
            _ => Step::Nok,
        }
    }

    fn to_mdast(&self, tokens: Vec<Token>) -> Node {
        Node::Custom(Custom {
            name: "lineBlock".into(),
            children: tokens
                .into_iter()
                .flat_map(|token| token.children)
                .collect(),
            ..Custom::default()
        })
    }
}

fn line_block(input: &str) -> Node {
    let parse = ParseOptions {
        document_constructs: vec![Box::new(LineBlock)],
        ..ParseOptions::default()
    };
    to_mdast(input, &parse).unwrap()
}

fn line_block_body(input: &str) -> Vec<Node> {
    match &line_block(input).children().unwrap()[0] {
        Node::Custom(node) => node.children.clone(),
        node => panic!("expected a line block, got {:?}", node),
    }
}

#[test]
fn containers_hold_flow() {
    assert!(
        matches!(&line_block_body("| a\n| b")[..], [Node::Paragraph(paragraph)] if paragraph.children.len() == 1),
        "should keep one paragraph across prefixes"
    );
    assert!(
        matches!(&line_block_body("| a\n| ===")[..], [Node::Heading(_)]),
        "should find a setext heading across prefixes"
    );
    assert!(
        matches!(&line_block_body("| - a\n| - b")[..], [Node::List(list)] if list.children.len() == 2),
        "should keep one list across prefixes"
    );
    assert!(
        matches!(&line_block_body("| a\nb")[..], [Node::Paragraph(_)]),
        "should continue a paragraph on a lazy line"
    );
    assert!(
        matches!(&line_block_body("| | a")[..], [Node::Custom(_)]),
        "should nest"
    );

    let tree = line_block("| a\n\nb");
    assert_eq!(
        tree.children().unwrap().len(),
        2,
        "should close at a line without the prefix"
    );
}

/// `==`, paired by the core like strikethrough.
struct Mark;

impl Construct for Mark {
    fn markers(&self) -> &[u8] {
        b"="
    }

    fn attention_sizes(&self) -> &[usize] {
        &[2]
    }

    fn step(&self, _: u16, _: &mut ConstructTokenizer) -> Step {
        unreachable!("should not step a delimiter run")
    }

    fn to_mdast(&self, tokens: Vec<Token>) -> Node {
        Node::Custom(Custom {
            name: "mark".into(),
            children: tokens
                .into_iter()
                .flat_map(|token| token.children)
                .collect(),
            ..Custom::default()
        })
    }
}

#[test]
fn pairs_delimiter_runs() {
    let parse = ParseOptions {
        text_constructs: vec![Box::new(Mark)],
        ..ParseOptions::default()
    };
    let cases = [
        ("==a *b*==", "<p><mark>a <em>b</em></mark></p>"),
        ("*==a==*", "<p><em><mark>a</mark></em></p>"),
        ("==a *b== c*", "<p><mark>a *b</mark> c*</p>"),
        ("===a===", "<p>===a===</p>"),
        ("== a ==", "<p>== a ==</p>"),
    ];

    for (input, expected) in cases {
        let tree = to_mdast(input, &parse).unwrap();
        let actual = render_marks(&tree);
        assert_eq!(actual, expected, "should pair runs in {:?}", input);
    }
}

/// Minimal HTML of paragraphs, emphasis, marks, and text.
fn render_marks(node: &Node) -> String {
    let inner = |children: &[Node]| children.iter().map(render_marks).collect::<String>();
    match node {
        Node::Root(root) => inner(&root.children),
        Node::Paragraph(paragraph) => format!("<p>{}</p>", inner(&paragraph.children)),
        Node::Emphasis(emphasis) => format!("<em>{}</em>", inner(&emphasis.children)),
        Node::Custom(custom) => format!("<mark>{}</mark>", inner(&custom.children)),
        node => node.to_string(),
    }
}

#[test]
fn leaves_container_prefixes_out_of_values() {
    let parse = ParseOptions {
        text_constructs: vec![Box::new(Scripted {
            marker: b'{',
            step: braces,
        })],
        document_constructs: vec![Box::new(LineBlock)],
        ..ParseOptions::default()
    };
    let tree = to_mdast("| {{a\n| b}}", &parse).unwrap();

    assert_eq!(
        find_scripted(&tree).and_then(|node| node.attributes.get("bracesData").cloned()),
        Some("a\nb".into()),
        "should leave out the prefix of a plugin container, like `> `"
    );
}
