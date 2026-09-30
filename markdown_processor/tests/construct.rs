use markdown::{
    extension::{Construct, ConstructTokenizer, ContentType, Step, Token},
    mdast,
};
use markdown_processor::Processor;
use pretty_assertions::assert_eq;
use std::borrow::Cow;

type StepFn = fn(u16, &mut ConstructTokenizer) -> Step;

/// A construct at `marker`, scripted by a function, whose custom node is
/// called `name` and holds the children of its content.
struct Scripted {
    name: &'static str,
    marker: u8,
    step: StepFn,
    continuation: Option<u16>,
}

impl Construct for Scripted {
    fn markers(&self) -> &[u8] {
        core::slice::from_ref(&self.marker)
    }

    fn step(&self, state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
        (self.step)(state, tokenizer)
    }

    fn continuation(&self) -> Option<u16> {
        self.continuation
    }

    fn to_mdast(&self, tokens: Vec<Token>) -> mdast::Node {
        mdast::Node::Custom(mdast::Custom {
            name: self.name.into(),
            children: tokens
                .into_iter()
                .flat_map(|token| token.children)
                .collect(),
            ..mdast::Custom::default()
        })
    }
}

/// `@` and a lowercase word, the word as text.
fn mention(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (0, Some(b'@')) => {
            tokenizer.enter("mention");
            tokenizer.consume();
            Step::Next(1)
        }
        (1, Some(b'a'..=b'z')) => {
            tokenizer.enter_content("mentionName", ContentType::Text);
            tokenizer.consume();
            Step::Next(2)
        }
        (2, Some(b'a'..=b'z')) => {
            tokenizer.consume();
            Step::Next(2)
        }
        (2, _) => {
            tokenizer.exit("mentionName");
            tokenizer.exit("mention");
            Step::Ok
        }
        _ => Step::Nok,
    }
}

/// `%` and the rest of the line as text.
fn caption(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (0, Some(b'%')) => {
            tokenizer.enter("caption");
            tokenizer.consume();
            Step::Next(1)
        }
        (1, Some(byte)) if byte != b'\n' => {
            tokenizer.enter_content("captionText", ContentType::Text);
            tokenizer.consume();
            Step::Next(2)
        }
        (2, None | Some(b'\n')) => {
            tokenizer.exit("captionText");
            tokenizer.exit("caption");
            Step::Ok
        }
        (2, Some(_)) => {
            tokenizer.consume();
            Step::Next(2)
        }
        _ => Step::Nok,
    }
}

/// `|` and an optional space before each line of a container, checked from
/// state 10 on later lines.
fn aside(state: u16, tokenizer: &mut ConstructTokenizer) -> Step {
    match (state, tokenizer.current()) {
        (0 | 10, Some(b'|')) => {
            if state == 0 {
                tokenizer.enter("aside");
            }
            tokenizer.enter("asidePrefix");
            tokenizer.consume();
            Step::Next(state + 1)
        }
        (1 | 11, Some(b' ')) => {
            tokenizer.consume();
            tokenizer.exit("asidePrefix");
            Step::Next(state + 1)
        }
        (1 | 11, _) => {
            tokenizer.exit("asidePrefix");
            Step::Retry(state + 1)
        }
        (2, _) => {
            tokenizer.enter_content("asideContent", ContentType::Document);
            Step::Ok
        }
        (12, _) => Step::Ok,
        _ => Step::Nok,
    }
}

fn scripted(name: &'static str, marker: u8, step: StepFn) -> Scripted {
    Scripted {
        name,
        marker,
        step,
        continuation: None,
    }
}

fn element(tag_name: impl Into<Cow<'static, str>>, children: Vec<hast::Node>) -> hast::Node {
    hast::Node::Element(hast::Element {
        tag_name: tag_name.into(),
        properties: vec![],
        children,
        position: None,
    })
}

#[test]
fn adds_constructs_to_text_flow_and_containers() {
    let processor = Processor::new().plugin(|processor: &mut Processor| {
        processor.add_text_construct(scripted("mention", b'@', mention));
        processor.add_flow_construct(scripted("caption", b'%', caption));
        processor.add_document_construct(Scripted {
            continuation: Some(10),
            ..scripted("aside", b'|', aside)
        });
        processor.add_hast_handler("mention", |_, children| vec![element("b", children)]);
        processor.add_hast_handler("caption", |_, children| {
            vec![element("figcaption", children)]
        });
        processor.add_hast_handler("aside", |_, children| vec![element("aside", children)]);
    });

    assert_eq!(
        processor.process("%a @b\n\n| c\n| d").unwrap(),
        "<figcaption>a <b>b</b></figcaption>\n<aside><p>c\nd</p></aside>",
        "should parse each kind of construct and render its node"
    );
}

#[test]
fn tries_constructs_in_the_order_added() {
    let processor = Processor::new().plugin(|processor: &mut Processor| {
        processor.add_text_construct(scripted("first", b'@', mention));
        processor.add_text_construct(scripted("second", b'@', mention));
        processor.add_flow_construct(scripted("first", b'%', caption));
        processor.add_flow_construct(scripted("second", b'%', caption));
        processor.add_document_construct(scripted("first", b'|', aside));
        processor.add_document_construct(scripted("second", b'|', aside));
        processor.add_hast_handler("first", |_, children| vec![element("b", children)]);
        processor.add_hast_handler("second", |_, children| vec![element("i", children)]);
    });

    assert_eq!(
        processor.process("@a\n\n%b\n\n| c").unwrap(),
        "<p><b>a</b></p>\n<b>b</b>\n<b><p>c</p></b>",
        "should try each kind of construct in the order added"
    );
}
