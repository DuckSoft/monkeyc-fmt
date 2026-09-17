use monkeyc_fmt::{Options, format};
use tree_sitter::{Node, Parser};

fn opts(width: usize) -> Options {
    Options {
        line_width: width,
        indent_width: 4,
    }
}

fn formatted(source: &str) -> String {
    format(source, &Options::default()).expect("valid Monkey C should format")
}

fn parse(source: &str) -> tree_sitter::Tree {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_monkeyc::LANGUAGE.into())
        .expect("Monkey C grammar loads");
    let tree = parser.parse(source, None).expect("parser returns a tree");
    assert!(
        !tree.root_node().has_error(),
        "fixture must parse: {source}"
    );
    tree
}

fn leaves<'a>(node: Node<'a>, source: &'a [u8], out: &mut Vec<(String, Vec<u8>)>) {
    if node.child_count() == 0 {
        out.push((node.kind().to_owned(), source[node.byte_range()].to_vec()));
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        leaves(child, source, out);
    }
}

fn assert_syntax_preserved(source: &str) -> String {
    let before_tree = parse(source);
    let output = formatted(source);
    let after_tree = parse(&output);
    assert_eq!(
        before_tree.root_node().to_sexp(),
        after_tree.root_node().to_sexp()
    );

    let mut before = Vec::new();
    let mut after = Vec::new();
    leaves(before_tree.root_node(), source.as_bytes(), &mut before);
    leaves(after_tree.root_node(), output.as_bytes(), &mut after);
    assert_eq!(before, after, "formatting changed a token");
    assert_eq!(formatted(&output), output, "formatting is not idempotent");
    output
}

#[test]
fn formats_representative_statements_and_blocks() {
    let input = "function f(x){if(x){return 1;}else{return 2;}while(x){x-=1;}}";
    let expected = "function f(x) {\n    if (x) {\n        return 1;\n    } else {\n        return 2;\n    }\n    while (x) {\n        x -= 1;\n    }\n}\n";
    assert_eq!(formatted(input), expected);
}

#[test]
fn wraps_lists_only_when_the_width_requires_it() {
    let input = "function call(){consume(alpha,beta,gamma,delta,epsilon);}";
    let wide = format(input, &opts(100)).unwrap();
    let narrow = format(input, &opts(24)).unwrap();
    assert_eq!(
        wide,
        "function call() {\n    consume(alpha, beta, gamma, delta, epsilon);\n}\n"
    );
    assert_eq!(
        narrow,
        "function call() {\n    consume(\n        alpha,\n        beta,\n        gamma,\n        delta,\n        epsilon\n    );\n}\n"
    );
}

#[test]
fn comments_cannot_swallow_the_following_token() {
    let source =
        "function f(x){var a=x+// keep this operand separate\n1;return a/* left */+/* right */2;}";
    let output = assert_syntax_preserved(source);
    assert!(output.contains("// keep this operand separate\n"));
    assert!(output.contains("1;"));
}

#[test]
fn literal_contents_and_escape_bytes_are_preserved() {
    let source = "var text = \"first line\n  second\\n\\u0041\\\"\\\\\";\nvar chars = ['\\n', '\\u263a', '\\\\', '\\''];";
    let output = assert_syntax_preserved(source);
    for literal in [
        "\"first line\n  second\\n\\u0041\\\"\\\\\"",
        "'\\n'",
        "'\\u263a'",
        "'\\\\'",
        "'\\''",
    ] {
        assert!(
            output.contains(literal),
            "literal bytes changed: {literal:?}"
        );
    }
}

#[test]
fn nested_generic_closers_stay_distinct_from_right_shifts() {
    let source = "function shift(value as Array<Array<Number>>){var a=value[0][0]>>2;var b=a+8>/* split */>1;a>>=1;}";
    let output = assert_syntax_preserved(source);
    assert!(output.contains("Array<Array<Number>>"));
    assert!(output.contains("> /* split */ >"));
}

#[test]
fn adjacent_unary_operators_do_not_turn_into_updates() {
    let source = "function signs(a,b){var x=+ +a;var y=- -b;var z=a+ +b;var q=a- -b;}";
    let output = assert_syntax_preserved(source);
    assert!(!output.contains("++a"));
    assert!(!output.contains("--b"));
}

#[test]
fn nullable_cast_ternary_and_monkey_c_precedence_survive() {
    let source = "function precedence(value as Number?) as Number? {var a=first+second<<third;var b=first&second*third;var c=first|second+third^fourth;var d=first==second<third;return value!=null?(!~-value as Number):null;}";
    assert_syntax_preserved(source);
}

#[test]
fn formats_annotations_dictionaries_and_switches() {
    let source = "(:background,:typecheck([disableBackgroundCheck])) module Worker{(:test) function choose(value){var table={:ready=>true,\"other\"=>false};switch(value){case :ready:return table[:ready];default:return false;}}}";
    let output = assert_syntax_preserved(source);
    assert!(output.contains("(:background, :typecheck([disableBackgroundCheck]))"));
    assert!(output.contains(":ready => true"));
    assert!(output.contains("switch (value) {\n"));
    assert!(output.contains("case :ready:"));
    assert!(output.contains("default:"));
}

#[test]
fn distinct_corpus_derived_programs_are_idempotent_and_token_exact() {
    // Inlined so the installed crate's tests do not depend on a sibling grammar checkout.
    let fixtures = [
        "typedef Table as Dictionary<Symbol, Array<Dictionary<String, Number or Null>>>;\nvar entries as Table?;\nfunction pair(v as Number | String) as [Number, [String, Boolean?],] or Null { return null; }",
        "typedef Options as { :name as String, \"values\" as Array<Number>, -1 as Boolean, true as Number, 'x' as String, };\ntypedef Callback as Method(value as Array<Number>) as Number or Null;",
        "var values = [1, null, [true],];\nvar empty = [], dict = {}, bytes = []b;\nvar table = {:key => 1, \"x\" => [1, 2],};\nvar buffer = [0, 127, 255,]b;",
        "function control(limit) { for (var i = 0, j = limit; i < j; i++, --j) { if (i == 2) { continue; } else { break; } } do { limit--; } while (limit > 0); }",
        "function recover() { try { throw new Lang.Exception(\"failure\"); } catch (ex instanceof Lang.Exception) { throw ex; } finally { cleanup(); } }",
        "//! docs\n/* before */ function /* name */ f(/* arg */ x) /* return */ as Number { var a = [1, /* middle */ 2,]; // trailing\nreturn x /* operand */ + a[0]; } // EOF",
    ];

    for fixture in fixtures {
        assert_syntax_preserved(fixture);
    }
}

#[test]
fn rejects_invalid_syntax_and_invalid_options() {
    assert!(format("function broken( {", &Options::default()).is_err());
    assert!(format("var x = 1;", &opts(19)).is_err());
    assert!(
        format(
            "var x = 1;",
            &Options {
                line_width: 100,
                indent_width: 0
            }
        )
        .is_err()
    );
    assert!(
        format(
            "var x = 1;",
            &Options {
                line_width: 100,
                indent_width: 17
            }
        )
        .is_err()
    );
}

#[test]
fn empty_and_final_newline_contract() {
    assert_eq!(formatted(""), "");
    assert_eq!(formatted("var x=1;\n\n"), "var x = 1;\n");
}

#[test]
fn line_comments_before_type_delimiters_do_not_hide_the_delimiter() {
    assert_syntax_preserved(
        "typedef T as Array // type argument\n<Number>;\n\
         typedef I as interface // members\n{function f();};",
    );
}

#[test]
fn nested_multiline_tokens_keep_their_original_line_endings_and_indentation() {
    let source = "class C { function f() { var text = \"one\r\n two\n\\n三\"; /* first\r\n   second */ return text; } }";
    let output = assert_syntax_preserved(source);
    assert!(output.contains("\"one\r\n two\n\\n三\""));
    assert!(output.contains("/* first\r\n   second */"));
}
