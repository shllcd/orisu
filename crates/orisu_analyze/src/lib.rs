use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_semantic::{Scoping, SemanticBuilder};
use oxc_span::{SourceType, Span};
use oxc_syntax::reference::ReferenceId;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::FxHashMap;
use std::path::Path;

/// Functions whose arguments are class lists.
pub const DEFAULT_CLASS_FUNCTIONS: &[&str] = &["cn", "clsx", "cx", "classnames", "classNames", "twMerge", "twJoin"];

/// Attributes and properties whose values are class lists.
pub const DEFAULT_CLASS_ATTRIBUTES: &[&str] =
    &["class", "className", "ngClass", "[ngClass]", "class:list", ":class", "v-bind:class", "[class]"];

/// The ending of attributes and properties that also hold class lists, like `buttonClassName`.
pub const CLASS_ATTRIBUTE_SUFFIX: &str = "ClassName";

/// Libraries that merge classes by their names, which renamed classes no longer carry.
const NAME_MERGERS: &[&str] = &["tailwind-merge", "tailwind-variants"];

const CLASS_LIST_METHODS: &[&str] = &["add", "remove", "toggle", "replace", "contains"];

const NO_TOUCH: (bool, bool) = (false, false);

/// Class positions beyond the built-in ones.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Functions whose arguments are class lists, besides [`DEFAULT_CLASS_FUNCTIONS`].
    pub functions: Vec<String>,
    /// Attributes and properties whose values are class lists, besides [`DEFAULT_CLASS_ATTRIBUTES`] and those ending
    /// in [`CLASS_ATTRIBUTE_SUFFIX`].
    pub attributes: Vec<String>,
}

/// A class at `start..end` of the source that can be rewritten. `quote` marks an identifier key, which must become a
/// string key to hold any class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub start: u32,
    pub end: u32,
    pub quote: bool,
}

/// The visible part of a class built at runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fragment {
    Prefix(String),
    Suffix(String),
    Infix(String),
}

impl Fragment {
    /// Whether `class` can be the runtime value this fragment is part of.
    pub fn matches(&self, class: &str) -> bool {
        match self {
            Fragment::Prefix(p) => class.starts_with(p.as_str()),
            Fragment::Suffix(s) => class.ends_with(s.as_str()),
            Fragment::Infix(i) => class.contains(i.as_str()),
        }
    }
}

#[derive(Debug, Default)]
pub struct Analysis {
    /// Classes in class positions, in source order.
    pub tokens: Vec<Token>,
    /// Classes in class positions that stay as written: their source text differs from their value, or they hold a
    /// quote or backslash that would need escaping once a minifier picks other quotes.
    pub pinned: Vec<String>,
    /// Values of the strings outside class positions.
    pub strings: Vec<String>,
    pub fragments: Vec<Fragment>,
    /// The import of a library that merges classes by name, if the module has one.
    pub name_merger: Option<String>,
}

/// What an expression in a class position holds: a class list, or objects nested `n` deep whose values are class lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    List,
    Dict(u8),
}

/// Finds the class positions of a JavaScript or TypeScript module; `None` when it does not parse.
pub fn analyze(source: &str, path: &Path, options: &Options) -> Option<Analysis> {
    let mut source_type = SourceType::from_path(path).ok()?;
    if source_type.is_javascript() {
        source_type = source_type.with_jsx(true);
    }
    let alloc = Allocator::default();
    let parsed = Parser::new(&alloc, source, source_type).parse();
    if parsed.fatal_error || parsed.diagnostics.has_errors() {
        return None;
    }
    let semantic = SemanticBuilder::new().build(&parsed.program).semantic;
    let scoping = semantic.scoping();
    let mut class_consts = FxHashMap::default();
    loop {
        let mut walker = Walker {
            source,
            options,
            class_consts: &class_consts,
            uses: FxHashMap::default(),
            consts: Vec::new(),
            in_const: false,
            exported: false,
            out: Analysis::default(),
        };
        walker.visit_program(&parsed.program);
        let next = resolve_consts(scoping, &walker.consts, &walker.uses);
        if next == class_consts {
            return Some(walker.out);
        }
        class_consts = next;
    }
}

/// The local constants read only in class positions, all in the same way.
fn resolve_consts(scoping: &Scoping, consts: &[SymbolId], uses: &FxHashMap<ReferenceId, Mode>) -> FxHashMap<SymbolId, Mode> {
    let mut out = FxHashMap::default();
    for &symbol in consts {
        let mut mode = None;
        let mut class_only = true;
        for &reference in scoping.get_resolved_reference_ids(symbol) {
            if !scoping.get_reference(reference).is_value() {
                continue;
            }
            match (uses.get(&reference), mode) {
                (Some(&used), None) => mode = Some(used),
                (Some(&used), Some(seen)) if used == seen => {}
                _ => class_only = false,
            }
        }
        if class_only && let Some(mode) = mode {
            out.insert(symbol, mode);
        }
    }
    out
}

fn has_letter(text: &str) -> bool {
    text.bytes().any(|b| b.is_ascii_alphabetic())
}

fn callee_name<'a>(callee: &Expression<'a>) -> Option<&'a str> {
    match callee.get_inner_expression() {
        Expression::Identifier(id) => Some(id.name.as_str()),
        Expression::StaticMemberExpression(member) => Some(member.property.name.as_str()),
        _ => None,
    }
}

struct Walker<'s> {
    source: &'s str,
    options: &'s Options,
    class_consts: &'s FxHashMap<SymbolId, Mode>,
    uses: FxHashMap<ReferenceId, Mode>,
    consts: Vec<SymbolId>,
    in_const: bool,
    exported: bool,
    out: Analysis,
}

impl<'a> Walker<'_> {
    fn class_name(&self, name: &str) -> Option<Mode> {
        if DEFAULT_CLASS_ATTRIBUTES.contains(&name)
            || name.ends_with(CLASS_ATTRIBUTE_SUFFIX)
            || self.options.attributes.iter().any(|a| a == name)
        {
            Some(Mode::List)
        } else if name == "classNames" || name.ends_with("ClassNames") {
            Some(Mode::Dict(1))
        } else {
            None
        }
    }

    fn is_class_function(&self, name: &str) -> bool {
        DEFAULT_CLASS_FUNCTIONS.contains(&name) || self.options.functions.iter().any(|f| f == name)
    }

    fn fragment(&mut self, text: &str, left: bool, right: bool) {
        if text.is_empty() || !has_letter(text) {
            return;
        }
        let fragment = match (left, right) {
            (true, true) => Fragment::Infix(text.to_owned()),
            (true, false) => Fragment::Suffix(text.to_owned()),
            _ => Fragment::Prefix(text.to_owned()),
        };
        self.out.fragments.push(fragment);
    }

    /// Records the class list `value` written as `raw` from source offset `start`; `touch` tells whether runtime text
    /// joins it on the left and right.
    fn class_text(&mut self, start: u32, raw: &str, value: &str, touch: (bool, bool)) {
        let exact = raw == value;
        let mut tokens = value.split_ascii_whitespace().peekable();
        let mut first = true;
        while let Some(token) = tokens.next() {
            let offset = token.as_ptr() as usize - value.as_ptr() as usize;
            let left = first && touch.0 && offset == 0;
            let right = tokens.peek().is_none() && touch.1 && offset + token.len() == value.len();
            first = false;
            if left || right {
                self.fragment(token, left, right);
            } else if exact && !token.contains(['"', '\'', '`', '\\']) {
                let start = start + offset as u32;
                self.out.tokens.push(Token { start, end: start + token.len() as u32, quote: false });
            } else {
                self.out.pinned.push(token.to_owned());
            }
        }
    }

    fn class_string(&mut self, literal: &StringLiteral<'a>, touch: (bool, bool)) {
        let start = literal.span.start + 1;
        let raw = &self.source[start as usize..literal.span.end as usize - 1];
        self.class_text(start, raw, &literal.value, touch);
    }

    fn class_template(&mut self, template: &TemplateLiteral<'a>, touch: (bool, bool)) {
        let last = template.quasis.len().saturating_sub(1);
        for (i, quasi) in template.quasis.iter().enumerate() {
            let raw = &self.source[quasi.span.start as usize..quasi.span.end as usize];
            let value = quasi.value.cooked.as_deref().unwrap_or(raw);
            self.class_text(quasi.span.start, raw, value, (i > 0 || touch.0, i < last || touch.1));
        }
        for expression in &template.expressions {
            self.class_expr(expression, Mode::List);
        }
    }

    /// Records a string outside class positions with the fragments it leaves when runtime text joins it.
    fn plain_text(&mut self, value: &str, touch: (bool, bool)) {
        self.out.strings.push(value.to_owned());
        let (left, right) = touch;
        if !value.contains(|c: char| c.is_ascii_whitespace()) {
            if left || right {
                self.fragment(value, left, right);
            }
            return;
        }
        if left && let Some(run) = value.split_ascii_whitespace().next().filter(|run| value.starts_with(run)) {
            self.fragment(run, true, false);
        }
        if right && let Some(run) = value.split_ascii_whitespace().next_back().filter(|run| value.ends_with(run)) {
            self.fragment(run, false, true);
        }
    }

    fn plain_template(&mut self, template: &TemplateLiteral<'a>, touch: (bool, bool)) {
        let last = template.quasis.len().saturating_sub(1);
        for (i, quasi) in template.quasis.iter().enumerate() {
            let value = quasi.value.cooked.as_deref().unwrap_or(quasi.value.raw.as_str());
            self.plain_text(value, (i > 0 || touch.0, i < last || touch.1));
        }
        for expression in &template.expressions {
            self.visit_expression(expression);
        }
    }

    fn plain_concat(&mut self, expr: &Expression<'a>, touch: (bool, bool)) {
        match expr.get_inner_expression() {
            Expression::BinaryExpression(binary) if binary.operator == BinaryOperator::Addition => {
                self.plain_concat(&binary.left, (touch.0, true));
                self.plain_concat(&binary.right, (true, touch.1));
            }
            Expression::StringLiteral(literal) => self.plain_text(&literal.value, touch),
            Expression::TemplateLiteral(template) => self.plain_template(template, touch),
            expr => self.visit_expression(expr),
        }
    }

    fn use_reference(&mut self, id: &IdentifierReference<'a>, mode: Mode) {
        if let Some(reference) = id.reference_id.get() {
            self.uses.insert(reference, mode);
        }
    }

    fn class_member(&mut self, expr: &Expression<'a>, mode: Mode) {
        let mut depth: u8 = 0;
        let mut current = expr;
        loop {
            match current.get_inner_expression() {
                Expression::StaticMemberExpression(member) => current = &member.object,
                Expression::ComputedMemberExpression(member) => {
                    self.visit_expression(&member.expression);
                    current = &member.object;
                }
                Expression::Identifier(id) => {
                    let mode = match mode {
                        Mode::List => Mode::Dict(depth),
                        Mode::Dict(n) => Mode::Dict(n.saturating_add(depth)),
                    };
                    self.use_reference(id, mode);
                    return;
                }
                other => {
                    self.visit_expression(other);
                    return;
                }
            }
            depth = depth.saturating_add(1);
        }
    }

    fn class_keys(&mut self, object: &ObjectExpression<'a>) {
        for property in &object.properties {
            match property {
                ObjectPropertyKind::ObjectProperty(p) => {
                    match &p.key {
                        PropertyKey::StringLiteral(literal) if !p.computed => self.class_string(literal, NO_TOUCH),
                        PropertyKey::StaticIdentifier(id) if p.shorthand => self.out.pinned.push(id.name.to_string()),
                        PropertyKey::StaticIdentifier(id) => {
                            self.out.tokens.push(Token { start: id.span.start, end: id.span.end, quote: true });
                        }
                        key => match key.as_expression() {
                            Some(expr) if p.computed => self.class_expr(expr, Mode::List),
                            _ => self.visit_property_key(key),
                        },
                    }
                    self.visit_expression(&p.value);
                }
                ObjectPropertyKind::SpreadProperty(spread) => self.class_expr(&spread.argument, Mode::List),
            }
        }
    }

    fn class_values(&mut self, object: &ObjectExpression<'a>, depth: u8) {
        let inner = if depth <= 1 { Mode::List } else { Mode::Dict(depth - 1) };
        for property in &object.properties {
            match property {
                ObjectPropertyKind::ObjectProperty(p) => {
                    if p.computed || matches!(p.key, PropertyKey::StringLiteral(_)) {
                        self.visit_property_key(&p.key);
                    }
                    self.class_expr(&p.value, inner);
                }
                ObjectPropertyKind::SpreadProperty(spread) => self.class_expr(&spread.argument, Mode::Dict(depth)),
            }
        }
    }

    fn class_argument(&mut self, argument: &Argument<'a>) {
        match argument {
            Argument::SpreadElement(spread) => self.class_expr(&spread.argument, Mode::List),
            argument => match argument.as_expression() {
                Some(expr) => self.class_expr(expr, Mode::List),
                None => self.visit_argument(argument),
            },
        }
    }

    fn class_expr(&mut self, expr: &Expression<'a>, mode: Mode) {
        self.class_expr_touching(expr, mode, NO_TOUCH);
    }

    /// Like `class_expr`, where `touch` tells whether runtime text joins the expression on its left and right.
    fn class_expr_touching(&mut self, expr: &Expression<'a>, mode: Mode, touch: (bool, bool)) {
        match (expr.get_inner_expression(), mode) {
            (Expression::StringLiteral(literal), Mode::List) => self.class_string(literal, touch),
            (Expression::TemplateLiteral(template), Mode::List) => self.class_template(template, touch),
            (Expression::BinaryExpression(binary), Mode::List) if binary.operator == BinaryOperator::Addition => {
                self.class_expr_touching(&binary.left, mode, (touch.0, true));
                self.class_expr_touching(&binary.right, mode, (true, touch.1));
            }
            (Expression::ArrayExpression(array), Mode::List) => {
                for element in &array.elements {
                    match element {
                        ArrayExpressionElement::SpreadElement(spread) => self.class_expr(&spread.argument, Mode::List),
                        ArrayExpressionElement::Elision(_) => {}
                        element => {
                            if let Some(expr) = element.as_expression() {
                                self.class_expr(expr, Mode::List);
                            }
                        }
                    }
                }
            }
            (Expression::ObjectExpression(object), Mode::List) => self.class_keys(object),
            (Expression::ObjectExpression(object), Mode::Dict(depth)) => self.class_values(object, depth),
            (Expression::ConditionalExpression(conditional), _) => {
                self.visit_expression(&conditional.test);
                self.class_expr_touching(&conditional.consequent, mode, touch);
                self.class_expr_touching(&conditional.alternate, mode, touch);
            }
            (Expression::LogicalExpression(logical), _) => {
                if logical.operator == LogicalOperator::And {
                    self.visit_expression(&logical.left);
                } else {
                    self.class_expr_touching(&logical.left, mode, touch);
                }
                self.class_expr_touching(&logical.right, mode, touch);
            }
            (Expression::SequenceExpression(sequence), _) => {
                let (last, rest) = sequence.expressions.split_last().expect("a sequence has expressions");
                for expr in rest {
                    self.visit_expression(expr);
                }
                self.class_expr_touching(last, mode, touch);
            }
            (Expression::Identifier(id), _) => self.use_reference(id, mode),
            (member @ (Expression::StaticMemberExpression(_) | Expression::ComputedMemberExpression(_)), _) => {
                self.class_member(member, mode);
            }
            (expr, _) => self.visit_expression(expr),
        }
    }

    fn cva_config(&mut self, expr: &Expression<'a>) {
        let Expression::ObjectExpression(object) = expr.get_inner_expression() else {
            return self.visit_expression(expr);
        };
        for property in &object.properties {
            match property {
                ObjectPropertyKind::ObjectProperty(p) if !p.computed && p.key.static_name().as_deref() == Some("variants") => {
                    self.class_expr(&p.value, Mode::Dict(2));
                }
                property => self.visit_object_property_kind(property),
            }
        }
    }

    fn is_class_list_call(callee: &Expression<'a>) -> Option<&'a str> {
        let Expression::StaticMemberExpression(method) = callee.get_inner_expression() else { return None };
        let Expression::StaticMemberExpression(list) = method.object.get_inner_expression() else { return None };
        let name = method.property.name.as_str();
        (list.property.name == "classList" && CLASS_LIST_METHODS.contains(&name)).then_some(name)
    }
}

impl<'a> Visit<'a> for Walker<'_> {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if let Some(method) = Self::is_class_list_call(&call.callee) {
            self.visit_expression(&call.callee);
            for (i, argument) in call.arguments.iter().enumerate() {
                if method == "toggle" && i > 0 {
                    self.visit_argument(argument);
                } else {
                    self.class_argument(argument);
                }
            }
            return;
        }
        match callee_name(&call.callee) {
            Some(name) if self.is_class_function(name) => {
                self.visit_expression(&call.callee);
                for argument in &call.arguments {
                    self.class_argument(argument);
                }
            }
            Some("cva") => {
                self.visit_expression(&call.callee);
                for (i, argument) in call.arguments.iter().enumerate() {
                    match (i, argument.as_expression()) {
                        (0, _) => self.class_argument(argument),
                        (1, Some(config)) => self.cva_config(config),
                        _ => self.visit_argument(argument),
                    }
                }
            }
            Some("setAttribute")
                if matches!(call.callee.get_inner_expression(), Expression::StaticMemberExpression(_))
                    && matches!(call.arguments.first(), Some(Argument::StringLiteral(name)) if name.value == "class") =>
            {
                self.visit_expression(&call.callee);
                for (i, argument) in call.arguments.iter().enumerate() {
                    if i == 1 {
                        self.class_argument(argument);
                    } else {
                        self.visit_argument(argument);
                    }
                }
            }
            _ => walk::walk_call_expression(self, call),
        }
    }

    fn visit_jsx_attribute(&mut self, attribute: &JSXAttribute<'a>) {
        let mode = match &attribute.name {
            JSXAttributeName::Identifier(id) => self.class_name(&id.name),
            JSXAttributeName::NamespacedName(_) => None,
        };
        let Some(mode) = mode else {
            return walk::walk_jsx_attribute(self, attribute);
        };
        match &attribute.value {
            Some(JSXAttributeValue::StringLiteral(literal)) if mode == Mode::List => self.class_string(literal, NO_TOUCH),
            Some(JSXAttributeValue::ExpressionContainer(container)) => match container.expression.as_expression() {
                Some(expr) => self.class_expr(expr, mode),
                None => self.visit_jsx_expression_container(container),
            },
            Some(value) => self.visit_jsx_attribute_value(value),
            None => {}
        }
    }

    fn visit_object_property(&mut self, property: &ObjectProperty<'a>) {
        let mode = if property.computed { None } else { property.key.static_name().and_then(|name| self.class_name(&name)) };
        match mode {
            Some(mode) => self.class_expr(&property.value, mode),
            None => walk::walk_object_property(self, property),
        }
    }

    fn visit_assignment_expression(&mut self, assignment: &AssignmentExpression<'a>) {
        if let AssignmentTarget::StaticMemberExpression(member) = &assignment.left
            && matches!(assignment.operator, AssignmentOperator::Assign | AssignmentOperator::Addition)
            && let Some(mode) = self.class_name(&member.property.name)
        {
            self.visit_expression(&member.object);
            let touch = (assignment.operator == AssignmentOperator::Addition, false);
            return self.class_expr_touching(&assignment.right, mode, touch);
        }
        walk::walk_assignment_expression(self, assignment);
    }

    fn visit_assignment_pattern(&mut self, pattern: &AssignmentPattern<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &pattern.left
            && let Some(mode) = self.class_name(&id.name)
        {
            self.visit_binding_pattern(&pattern.left);
            return self.class_expr(&pattern.right, mode);
        }
        walk::walk_assignment_pattern(self, pattern);
    }

    fn visit_binding_property(&mut self, property: &BindingProperty<'a>) {
        if !property.computed
            && let BindingPattern::AssignmentPattern(pattern) = &property.value
            && let Some(mode) = property.key.static_name().and_then(|name| self.class_name(&name))
        {
            self.visit_binding_pattern(&pattern.left);
            return self.class_expr(&pattern.right, mode);
        }
        walk::walk_binding_property(self, property);
    }

    fn visit_variable_declaration(&mut self, declaration: &VariableDeclaration<'a>) {
        let outer = self.in_const;
        self.in_const = declaration.kind == VariableDeclarationKind::Const;
        walk::walk_variable_declaration(self, declaration);
        self.in_const = outer;
    }

    fn visit_variable_declarator(&mut self, declarator: &VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &declarator.id
            && let (Some(symbol), Some(init)) = (id.symbol_id.get(), &declarator.init)
        {
            if self.in_const && !self.exported {
                self.consts.push(symbol);
            }
            if let Some(&mode) = self.class_consts.get(&symbol) {
                return self.class_expr(init, mode);
            }
        }
        walk::walk_variable_declarator(self, declarator);
    }

    fn visit_export_declaration(&mut self, export: &ExportDeclaration<'a>) {
        let outer = self.exported;
        self.exported = true;
        self.visit_declaration(&export.declaration);
        self.exported = outer;
    }

    fn visit_export_from_declaration(&mut self, _: &ExportFromDeclaration<'a>) {}

    fn visit_export_all_declaration(&mut self, _: &ExportAllDeclaration<'a>) {}

    fn visit_import_declaration(&mut self, import: &ImportDeclaration<'a>) {
        let source = import.source.value.as_str();
        if NAME_MERGERS.iter().any(|lib| source == *lib || source.strip_prefix(lib).is_some_and(|rest| rest.starts_with('/'))) {
            self.out.name_merger = Some(source.to_owned());
        }
    }

    fn visit_import_expression(&mut self, import: &ImportExpression<'a>) {
        if !matches!(import.source, Expression::StringLiteral(_)) {
            self.visit_expression(&import.source);
        }
        if let Some(options) = &import.options {
            self.visit_expression(options);
        }
    }

    fn visit_directive(&mut self, _: &Directive<'a>) {}

    fn visit_ts_type(&mut self, _: &TSType<'a>) {}

    fn visit_string_literal(&mut self, literal: &StringLiteral<'a>) {
        self.out.strings.push(literal.value.to_string());
    }

    fn visit_template_literal(&mut self, template: &TemplateLiteral<'a>) {
        self.plain_template(template, NO_TOUCH);
    }

    fn visit_binary_expression(&mut self, binary: &BinaryExpression<'a>) {
        if binary.operator == BinaryOperator::Addition {
            self.plain_concat(&binary.left, (false, true));
            self.plain_concat(&binary.right, (true, false));
        } else {
            walk::walk_binary_expression(self, binary);
        }
    }

    fn visit_reg_exp_literal(&mut self, regex: &RegExpLiteral<'a>) {
        let Span { start, end, .. } = regex.span;
        self.out.strings.push(self.source[start as usize..end as usize].to_owned());
    }
}
