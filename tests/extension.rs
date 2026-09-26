use markdown::{
    extension::{ConstructTokenizer, Step, TextConstruct, Token},
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

impl TextConstruct for Mention {
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

    fn to_mdast(&self, tokens: &[Token]) -> Node {
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
            .map(|construct| Box::new(construct) as Box<dyn TextConstruct>)
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

impl TextConstruct for Scripted {
    fn markers(&self) -> &[u8] {
        core::slice::from_ref(&self.marker)
    }

    fn step(&self, state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        (self.step)(state, tokenizer)
    }

    fn to_mdast(&self, tokens: &[Token]) -> Node {
        Node::Custom(Custom {
            name: "scripted".into(),
            value: Some(tokens[0].value.clone().into_owned()),
            attributes: tokens[1..]
                .iter()
                .map(|token| (token.name.into(), token.value.clone().into_owned()))
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
        Node::Custom(custom) => Some(custom),
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
fn does_not_end_on_a_line_ending_one_step_later() {
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
        vec!["%b"],
        "should reject the first match, and keep the later one"
    );
}

#[test]
fn rejects_tokens_that_stay_empty_up_to_a_line_ending() {
    let tree = to_mdast("> {{\n> }}", &scripted(braces)).unwrap();

    assert!(
        find_scripted(&tree).is_none(),
        "should treat a token closed right after a line ending as empty, like one closed on its line"
    );
}

#[test]
fn starts_empty_tokens_after_a_line_ending() {
    for input in ["> {{\n> a}}", "> {{\r\n> a}}"] {
        let tree = to_mdast(input, &scripted(braces)).unwrap();
        let node = find_scripted(&tree).expect("expected a match");

        assert_eq!(
            node.attributes.get("bracesData").map(String::as_str),
            Some("a"),
            "should start a token that is empty at a line ending after it, in {:?}",
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
