use mark::Mark;
use markdown_processor::Processor;
use pretty_assertions::assert_eq;

fn html(input: &str) -> String {
    let mut processor = Processor::new().plugin(Mark);
    processor.parse.constructs.gfm_strikethrough = true;
    processor.process(input).unwrap()
}

#[test]
fn marks_text() {
    let cases = [
        ("a ==b== c", "<p>a <mark>b</mark> c</p>"),
        ("==a *b* c==", "<p><mark>a <em>b</em> c</mark></p>"),
        ("*==a==*", "<p><em><mark>a</mark></em></p>"),
        ("==a ~~b~~==", "<p><mark>a <del>b</del></mark></p>"),
        ("[==a==](b)", "<p><a href=\"b\"><mark>a</mark></a></p>"),
        (
            "> ==a\n> b==",
            "<blockquote>\n<p><mark>a\nb</mark></p>\n</blockquote>",
        ),
        ("a==b==c", "<p>a<mark>b</mark>c</p>"),
    ];
    for (input, expected) in cases {
        assert_eq!(html(input), expected, "should mark {:?}", input);
    }
}

#[test]
fn pairs_like_strikethrough() {
    let cases = [
        ("==a *b== c*", "<p><mark>a *b</mark> c*</p>"),
        ("=a=", "<p>=a=</p>"),
        ("===a===", "<p>===a===</p>"),
        ("== a ==", "<p>== a ==</p>"),
        ("==a", "<p>==a</p>"),
    ];
    for (input, expected) in cases {
        assert_eq!(
            html(input),
            expected,
            "should not cross or mismatch in {:?}",
            input
        );
    }
}
