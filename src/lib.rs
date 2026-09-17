//! Syntax-preserving Monkey C formatting, independent of any editor or SDK.
//!
//! Only whitespace between syntax tokens is changed. Comments, literal contents,
//! parentheses, and punctuation are retained. The result is reparsed and compared
//! with the input tree before it is returned to a caller.

use pretty::{Arena, DocAllocator, DocBuilder};
use std::fmt;
use tree_sitter::{Node, Parser};

/// Formatting policy. Width is a soft target: literals and comments are never reflowed.
#[derive(Clone, Debug)]
pub struct Options {
    pub line_width: usize,
    pub indent_width: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            line_width: 100,
            indent_width: 4,
        }
    }
}

/// An error never contains a partially formatted result.
#[derive(Debug)]
pub enum FormatError {
    InvalidOptions(&'static str),
    Syntax { line: usize, column: usize },
    Nesting { line: usize, column: usize },
    Internal(&'static str),
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOptions(message) | Self::Internal(message) => f.write_str(message),
            Self::Syntax { line, column } => {
                write!(f, "syntax error at {line}:{column}; refusing to format")
            }
            Self::Nesting { line, column } => {
                write!(f, "syntax nesting exceeds 256 levels at {line}:{column}")
            }
        }
    }
}

impl std::error::Error for FormatError {}

/// Format UTF-8 Monkey C source using the pinned Tree-sitter grammar.
///
/// Indentation must be 1–16 spaces and the line width at least 20 columns.
/// Output uses LF between tokens; line endings inside literals/comments are
/// untouched. Empty or whitespace-only input becomes empty output. Nonempty
/// output has one final newline. Extremely deep syntax is rejected rather than
/// risking stack exhaustion during document construction.
pub fn format(source: &str, options: &Options) -> Result<String, FormatError> {
    if options.line_width < 20 {
        return Err(FormatError::InvalidOptions(
            "line width must be at least 20",
        ));
    }
    if !(1..=16).contains(&options.indent_width) {
        return Err(FormatError::InvalidOptions(
            "indent width must be between 1 and 16",
        ));
    }
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_monkeyc::LANGUAGE.into())
        .map_err(|_| FormatError::Internal("unable to load Monkey C grammar"))?;
    let tree = parser
        .parse(source, None)
        .ok_or(FormatError::Internal("unable to parse source"))?;
    validate(tree.root_node())?;
    if tree.root_node().child_count() == 0 {
        return Ok(String::new());
    }
    let arena = Arena::new();
    let printer = Printer {
        source,
        arena: &arena,
        indent: options.indent_width as isize,
    };
    let document = printer.node(tree.root_node()).append(arena.hardline());
    let mut output = String::with_capacity(source.len());
    document
        .render_fmt(options.line_width, &mut output)
        .map_err(|_| FormatError::Internal("unable to render formatted source"))?;
    let formatted = parser
        .parse(&output, None)
        .ok_or(FormatError::Internal("unable to parse formatted source"))?;
    if formatted.root_node().has_error()
        || !equivalent(tree.root_node(), formatted.root_node(), source, &output)
    {
        return Err(FormatError::Internal(
            "formatting would change the syntax tree; refusing to return unsafe output",
        ));
    }
    Ok(output)
}

fn validate(root: Node<'_>) -> Result<(), FormatError> {
    let mut cursor = root.walk();
    let mut depth = 0;
    loop {
        let node = cursor.node();
        let point = node.start_position();
        if node.is_error() || node.is_missing() {
            return Err(FormatError::Syntax {
                line: point.row + 1,
                column: point.column + 1,
            });
        }
        if depth > 256 {
            return Err(FormatError::Nesting {
                line: point.row + 1,
                column: point.column + 1,
            });
        }
        if cursor.goto_first_child() {
            depth += 1;
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return Ok(());
            }
            depth -= 1;
        }
    }
}

fn equivalent(before: Node<'_>, after: Node<'_>, source: &str, output: &str) -> bool {
    let mut left = before.walk();
    let mut right = after.walk();
    loop {
        let a = left.node();
        let b = right.node();
        if a.kind_id() != b.kind_id()
            || a.is_named() != b.is_named()
            || left.field_id() != right.field_id()
            || a.child_count() != b.child_count()
        {
            return false;
        }
        if a.child_count() == 0 && source[a.byte_range()] != output[b.byte_range()] {
            return false;
        }
        if left.goto_first_child() {
            if !right.goto_first_child() {
                return false;
            }
            continue;
        }
        loop {
            if left.goto_next_sibling() {
                if !right.goto_next_sibling() {
                    return false;
                }
                break;
            }
            if !left.goto_parent() {
                return true;
            }
            if !right.goto_parent() {
                return false;
            }
        }
    }
}

type Doc<'a> = DocBuilder<'a, Arena<'a>>;

struct Printer<'a> {
    source: &'a str,
    arena: &'a Arena<'a>,
    indent: isize,
}

impl<'a> Printer<'a> {
    fn text(&self, node: Node<'_>) -> &'a str {
        &self.source[node.byte_range()]
    }

    fn literal(&self, text: &'a str) -> Doc<'a> {
        if !text.contains('\n') {
            return self.arena.text(text);
        }
        // Reset only the indentation of the raw token's internal newlines.
        // Adding ambient indentation would change multiline string values and
        // mutate comment text. Hardlines also keep containing groups from flattening.
        let arena = self.arena;
        arena.nesting(move |level| {
            arena
                .intersperse(
                    text.split('\n').map(|part| arena.text(part)),
                    arena.hardline(),
                )
                .nest(-(level as isize))
                .into_doc()
        })
    }

    fn node(&self, node: Node<'_>) -> Doc<'a> {
        let kind = node.kind();
        if node.child_count() == 0
            || matches!(
                kind,
                "string"
                    | "character"
                    | "identifier"
                    | "number"
                    | "comment"
                    | "boolean"
                    | "null"
                    | "self"
                    | "global"
                    | "modifier"
            )
        {
            return self.literal(self.text(node));
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.children(&mut cursor).collect();
        match kind {
            "source_file" => self.vertical(&children),
            "block" | "class_body" | "module_body" | "switch_body" | "enum_body" => {
                self.body(&children)
            }
            "interface_type" => {
                let open = children.iter().position(|n| n.kind() == "{").unwrap();
                self.inline(kind, &children[..open])
                    .append(self.separator(kind, children[open - 1], children[open]))
                    .append(self.body(&children[open..]))
            }
            "switch_case" | "switch_default" => self.case(kind, &children),
            "parameters"
            | "arguments"
            | "parenthesized_expression"
            | "tuple_type"
            | "array_expression"
            | "byte_array_expression"
            | "annotation_arguments"
            | "annotation_array"
            | "annotations" => self.delimited(kind, &children, false),
            "dictionary_expression" | "dictionary_type" => self.delimited(kind, &children, true),
            "generic_type" => {
                let open = children.iter().position(|n| n.kind() == "<").unwrap();
                self.inline(kind, &children[..open])
                    .append(self.separator(kind, children[open - 1], children[open]))
                    .append(self.delimited(kind, &children[open..], false))
            }
            "method_type" if children[0].kind() == "(" => self.delimited(kind, &children, false),
            "for_statement" | "catch_clause" => self.control(kind, &children),
            "binary_expression" | "union_type" | "conditional_expression" => {
                self.expression(kind, &children)
            }
            _ => self.inline(kind, &children),
        }
    }

    fn gap(&self, previous: Node<'_>, next: Node<'_>) -> &'a str {
        &self.source[previous.end_byte()..next.start_byte()]
    }

    fn same_line(&self, previous: Node<'_>, next: Node<'_>) -> bool {
        !self.gap(previous, next).contains(['\n', '\r'])
    }

    fn blank_line(&self, previous: Node<'_>, next: Node<'_>) -> bool {
        let gap = self.gap(previous, next);
        let newline = if gap.contains('\n') { b'\n' } else { b'\r' };
        gap.bytes().filter(|b| *b == newline).take(2).count() == 2
    }

    fn ends_line_comment(&self, mut node: Node<'_>) -> bool {
        while node.child_count() > 0 && !matches!(node.kind(), "string" | "character" | "comment") {
            node = node.child(node.child_count() - 1).unwrap();
        }
        node.kind() == "comment" && self.text(node).starts_with("//")
    }

    fn hard_separator(&self, previous: Node<'_>, next: Node<'_>) -> Doc<'a> {
        let line = self.arena.hardline();
        if self.blank_line(previous, next) {
            line.append(self.arena.hardline())
        } else {
            line
        }
    }

    fn vertical(&self, children: &[Node<'_>]) -> Doc<'a> {
        let mut doc = self.arena.nil();
        let mut previous = None;
        for &child in children {
            if let Some(prev) = previous {
                let separator = if self.ends_line_comment(prev) {
                    self.hard_separator(prev, child)
                } else if child.kind() == "," {
                    self.arena.nil()
                } else if child.kind() == "comment" && self.same_line(prev, child) {
                    self.arena.space()
                } else {
                    self.hard_separator(prev, child)
                };
                doc = doc.append(separator);
            }
            doc = doc.append(self.node(child));
            previous = Some(child);
        }
        doc
    }

    fn body(&self, children: &[Node<'_>]) -> Doc<'a> {
        let first = children[0];
        let last = children[children.len() - 1];
        if children.len() == 2 {
            return self.node(first).append(self.node(last));
        }
        self.node(first)
            .append(
                self.arena
                    .hardline()
                    .append(self.vertical(&children[1..children.len() - 1]))
                    .nest(self.indent),
            )
            .append(self.arena.hardline())
            .append(self.node(last))
    }

    fn case(&self, kind: &str, children: &[Node<'_>]) -> Doc<'a> {
        let colon = children.iter().position(|n| n.kind() == ":").unwrap();
        let header = self.inline(kind, &children[..=colon]);
        if colon + 1 == children.len() {
            return header;
        }
        let rest = &children[colon + 1..];
        // A trailing label comment belongs on its original line. The remaining
        // statements still get a full case-body indentation level.
        if rest[0].kind() == "comment" && self.same_line(children[colon], rest[0]) {
            let header = header.append(" ").append(self.node(rest[0]));
            if rest.len() == 1 {
                return header;
            }
            return header.append(
                self.arena
                    .hardline()
                    .append(self.vertical(&rest[1..]))
                    .nest(self.indent),
            );
        }
        header.append(
            self.arena
                .hardline()
                .append(self.vertical(rest))
                .nest(self.indent),
        )
    }

    fn delimited(&self, kind: &str, children: &[Node<'_>], spaced: bool) -> Doc<'a> {
        let last = children.len() - 1;
        let open = self.node(children[0]);
        let close = self.node(children[last]);
        if last == 1 {
            return open.append(close);
        }
        let inner = &children[1..last];
        let edge = || {
            if spaced {
                self.arena.line()
            } else {
                self.arena.line_()
            }
        };
        let closing = if self.ends_line_comment(inner[inner.len() - 1]) {
            self.arena.hardline()
        } else {
            edge()
        };
        open.append(edge().append(self.sequence(kind, inner)).nest(self.indent))
            .append(closing)
            .append(close)
            .group()
    }

    fn control(&self, kind: &str, children: &[Node<'_>]) -> Doc<'a> {
        let open = children.iter().position(|n| n.kind() == "(").unwrap();
        let close = children.iter().rposition(|n| n.kind() == ")").unwrap();
        let mut doc = self.inline(kind, &children[..open]);
        doc = doc.append(self.separator(kind, children[open - 1], children[open]));
        doc = doc.append(self.delimited(kind, &children[open..=close], false));
        if close + 1 < children.len() {
            doc = doc
                .append(self.separator(kind, children[close], children[close + 1]))
                .append(self.inline(kind, &children[close + 1..]));
        }
        doc
    }

    fn inline(&self, kind: &str, children: &[Node<'_>]) -> Doc<'a> {
        // Group headers separately from bodies, whose hardlines must not force
        // every optional break in a function signature or condition.
        let body = children.iter().position(|n| {
            matches!(
                n.kind(),
                "block" | "class_body" | "module_body" | "enum_body" | "switch_body"
            )
        });
        if let Some(index) = body.filter(|index| *index > 0) {
            self.sequence(kind, &children[..index])
                .group()
                .append(self.separator(kind, children[index - 1], children[index]))
                .append(self.sequence(kind, &children[index..]))
        } else {
            self.sequence(kind, children).group()
        }
    }

    fn sequence(&self, kind: &str, children: &[Node<'_>]) -> Doc<'a> {
        let mut doc = self.arena.nil();
        for (index, &child) in children.iter().enumerate() {
            let part = self.node(child);
            if index == 0 {
                doc = doc.append(part);
            } else {
                let prev = children[index - 1];
                let separator = self.separator(kind, prev, child);
                if prev.kind() == ","
                    && matches!(
                        kind,
                        "variable_declaration"
                            | "local_variable_declaration"
                            | "for_variable_declaration"
                            | "expression_list"
                    )
                {
                    doc = doc.append(separator.append(part).nest(self.indent));
                } else {
                    doc = doc.append(separator).append(part);
                }
            }
        }
        doc
    }

    fn expression(&self, kind: &str, children: &[Node<'_>]) -> Doc<'a> {
        let mut tail = self.arena.nil();
        for (index, &child) in children.iter().enumerate().skip(1) {
            let prev = children[index - 1];
            let operator = if kind == "conditional_expression" {
                matches!(child.kind(), "?" | ":")
            } else {
                child.kind() != "comment"
                    && (!child.is_named() || child.kind() == "right_shift_operator")
            };
            let separator = if operator && prev.kind() != "comment" && !self.ends_line_comment(prev)
            {
                self.arena.line()
            } else {
                self.separator(kind, prev, child)
            };
            tail = tail.append(separator).append(self.node(child));
        }
        self.node(children[0])
            .append(tail.nest(self.indent))
            .group()
    }

    fn separator(&self, parent: &str, previous: Node<'_>, next: Node<'_>) -> Doc<'a> {
        let left = previous.kind();
        let right = next.kind();
        if self.ends_line_comment(previous) {
            return self.arena.hardline();
        }
        if left == "comment" || right == "comment" {
            return if self.same_line(previous, next) {
                self.arena.space()
            } else {
                self.hard_separator(previous, next)
            };
        }
        if left == "annotations" {
            return self.arena.hardline();
        }
        if matches!(right, "," | ";" | ")" | "]" | "]b")
            || left == "."
            || right == "."
            || matches!(right, "parameters" | "arguments")
            || (right == ":" && matches!(parent, "switch_case" | "switch_default"))
            || (right == "<" && parent == "generic_type")
        {
            return self.arena.nil();
        }
        if left == "," || (left == ";" && parent == "for_statement") {
            return self.arena.line();
        }
        if matches!(left, "(" | "[") {
            return self.arena.nil();
        }
        if right == "(" {
            return if matches!(left, "if" | "while" | "for" | "switch" | "catch") {
                self.arena.space()
            } else {
                self.arena.nil()
            };
        }
        if right == "[" {
            return if left == "new" {
                self.arena.space()
            } else {
                self.arena.nil()
            };
        }
        if matches!(
            parent,
            "symbol"
                | "annotation"
                | "nullable_type"
                | "right_shift_operator"
                | "update_expression"
                | "signed_number"
        ) {
            return self.arena.nil();
        }
        if parent == "unary_expression" {
            // Without this gap, - -value and + +value become update tokens.
            let a = self.text(previous).as_bytes().last();
            let b = self.text(next).as_bytes().first();
            return if a == b && matches!(a, Some(b'+' | b'-')) {
                self.arena.space()
            } else {
                self.arena.nil()
            };
        }
        self.arena.space()
    }
}
