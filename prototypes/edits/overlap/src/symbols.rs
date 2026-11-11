//! Symbols: the functions, methods and types of a file, found with a tree-sitter grammar of its
//! language, each named with the symbols that enclose it.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use tree_sitter::{Node, Parser};

use crate::Result;

/// A language whose symbols the prototype finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    TypeScript,
    Python,
}

/// A symbol and the symbols that enclose it, outermost first: each one's kind of syntax node and
/// name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Symbol(pub Vec<(String, String)>);

/// `<kind> <name>` of each symbol, outermost first, joined by ` > `.
impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for (i, (kind, name)) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(" > ")?;
            }
            write!(f, "{kind} {name}")?;
        }
        Ok(())
    }
}

impl Language {
    /// The language of the file at `path`, by its extension: `.rs`, `.ts` or `.py`.
    pub fn of(path: &str) -> Option<Self> {
        match Path::new(path).extension()?.to_str()? {
            "rs" => Some(Self::Rust),
            "ts" => Some(Self::TypeScript),
            "py" => Some(Self::Python),
            _ => None,
        }
    }

    fn grammar(self) -> tree_sitter::Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
        }
    }

    /// The name of `node`, a node of `source`, if it is a symbol: a function, method or type, or a
    /// node that holds them, such as a module or a Rust `impl` block.
    fn name(self, node: Node, source: &[u8]) -> Option<String> {
        let field = |name| {
            let child = node.child_by_field_name(name)?;
            Some(String::from_utf8_lossy(&source[child.byte_range()]).into_owned())
        };
        match (self, node.kind()) {
            (
                Self::Rust,
                "function_item"
                | "function_signature_item"
                | "struct_item"
                | "enum_item"
                | "union_item"
                | "trait_item"
                | "type_item"
                | "mod_item",
            )
            | (
                Self::TypeScript,
                "function_declaration"
                | "generator_function_declaration"
                | "class_declaration"
                | "abstract_class_declaration"
                | "method_definition"
                | "abstract_method_signature"
                | "method_signature"
                | "interface_declaration"
                | "enum_declaration"
                | "type_alias_declaration"
                | "internal_module",
            )
            | (Self::Python, "function_definition" | "class_definition") => field("name"),
            // An `impl` block, named by its type, and its trait if it has one.
            (Self::Rust, "impl_item") => {
                let name = field("type")?;
                Some(match field("trait") {
                    Some(trait_name) => format!("{trait_name} for {name}"),
                    None => name,
                })
            }
            // A variable or a class field that holds a function.
            (Self::TypeScript, "variable_declarator" | "public_field_definition") => {
                let value = node.child_by_field_name("value")?.kind();
                let function = [
                    "arrow_function",
                    "function_expression",
                    "generator_function",
                ];
                function.contains(&value).then(|| field("name"))?
            }
            _ => None,
        }
    }
}

/// The innermost symbol enclosing each line of `source`, a file in `language`: the first item for
/// its first line. A line outside every symbol has none.
pub fn innermost(language: Language, source: &[u8]) -> Result<Vec<Option<Symbol>>> {
    let mut parser = Parser::new();
    parser.set_language(&language.grammar())?;
    let tree = parser
        .parse(source, None)
        .ok_or("tree-sitter gave no tree")?;
    let lines = source.split_inclusive(|&byte| byte == b'\n').count();
    let mut symbols = vec![None; lines];
    visit(
        language,
        tree.root_node(),
        source,
        &mut Vec::new(),
        &mut symbols,
    );
    Ok(symbols)
}

/// Gives each line of `node` the innermost symbol that encloses it, below the symbols `enclosing`.
fn visit(
    language: Language,
    node: Node,
    source: &[u8],
    enclosing: &mut Vec<(String, String)>,
    symbols: &mut [Option<Symbol>],
) {
    let name = language.name(node, source);
    if let Some(name) = &name {
        enclosing.push((node.kind().to_owned(), name.clone()));
        let (first, last) = (node.start_position().row, node.end_position().row);
        // The symbols inside this one overwrite their lines next.
        for line in symbols.iter_mut().take(last + 1).skip(first) {
            *line = Some(Symbol(enclosing.clone()));
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(language, child, source, enclosing, symbols);
    }
    if name.is_some() {
        enclosing.pop();
    }
}

/// The innermost symbols enclosing `lines`, 1-based, of `source`, a file in `language`.
pub fn enclosing(
    language: Language,
    source: &[u8],
    lines: impl IntoIterator<Item = usize>,
) -> Result<BTreeSet<Symbol>> {
    let symbols = innermost(language, source)?;
    Ok(lines
        .into_iter()
        .filter_map(|line| symbols.get(line - 1).cloned().flatten())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Asserts that each of `lines`, the lines of a file in `language`, each with the innermost
    /// symbol enclosing it or `""` for none, gets that symbol.
    fn assert_symbols(language: Language, lines: &[(&str, &str)]) {
        let source: String = lines.iter().map(|(line, _)| format!("{line}\n")).collect();
        let symbols = innermost(language, source.as_bytes()).unwrap();
        assert_eq!(symbols.len(), lines.len(), "one item per line");
        let got: Vec<(&str, String)> = lines
            .iter()
            .zip(symbols)
            .map(|((line, _), symbol)| (*line, symbol.map(|s| s.to_string()).unwrap_or_default()))
            .collect();
        let want: Vec<(&str, String)> = lines
            .iter()
            .map(|(line, symbol)| (*line, symbol.to_string()))
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn languages_go_by_the_extension() {
        assert_eq!(Language::of("src/lib.rs"), Some(Language::Rust));
        assert_eq!(Language::of("web/app.ts"), Some(Language::TypeScript));
        assert_eq!(Language::of("tools/run.py"), Some(Language::Python));
        for path in [
            "src/main.go",
            "README.md",
            "Makefile",
            "src.rs/lib",
            "web/app.tsx",
        ] {
            assert_eq!(Language::of(path), None, "{path}");
        }
    }

    #[test]
    fn rust_symbols_are_functions_types_traits_impl_blocks_and_modules() {
        let store = "struct_item Store";
        let shelf = "trait_item Shelf";
        let imp = "impl_item Store";
        let reserve = "impl_item Store > function_item reserve";
        let display = "impl_item fmt::Display for Box<T>";
        let tests = "mod_item tests";
        assert_symbols(
            Language::Rust,
            &[
                ("//! Store.", ""),
                ("use std::fmt;", ""),
                ("", ""),
                ("/// A store.", ""),
                ("pub struct Store {", store),
                ("    items: u32,", store),
                ("}", store),
                ("pub enum Kind {", "enum_item Kind"),
                ("    Item,", "enum_item Kind"),
                ("}", "enum_item Kind"),
                ("pub union Bits { word: u32 }", "union_item Bits"),
                ("pub type Count = u32;", "type_item Count"),
                ("pub trait Shelf {", shelf),
                (
                    "    fn put(&mut self);",
                    "trait_item Shelf > function_signature_item put",
                ),
                (
                    "    fn take(&mut self) {",
                    "trait_item Shelf > function_item take",
                ),
                (
                    "        self.put();",
                    "trait_item Shelf > function_item take",
                ),
                ("    }", "trait_item Shelf > function_item take"),
                ("}", shelf),
                ("impl Store {", imp),
                ("    const LIMIT: u32 = 9;", imp),
                ("    /// Reserves one.", imp),
                ("    pub fn reserve(&mut self) {", reserve),
                (
                    "        fn inner() {}",
                    "impl_item Store > function_item reserve > function_item inner",
                ),
                ("        let take = |n: u32| n;", reserve),
                ("        self.items -= take(1);", reserve),
                ("    }", reserve),
                ("}", imp),
                ("impl<T> fmt::Display for Box<T> {", display),
                (
                    "    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {",
                    "impl_item fmt::Display for Box<T> > function_item fmt",
                ),
                (
                    "        write!(f, \"store\")",
                    "impl_item fmt::Display for Box<T> > function_item fmt",
                ),
                (
                    "    }",
                    "impl_item fmt::Display for Box<T> > function_item fmt",
                ),
                ("}", display),
                ("mod tests {", tests),
                ("    #[test]", tests),
                (
                    "    fn reserves() {}",
                    "mod_item tests > function_item reserves",
                ),
                ("}", tests),
                ("const LIMIT: u32 = 9;", ""),
            ],
        );
    }

    #[test]
    fn typescript_symbols_are_functions_classes_methods_types_namespaces_and_function_variables() {
        let store = "class_declaration Store";
        let reserve = "class_declaration Store > method_definition reserve";
        let release = "class_declaration Store > public_field_definition release";
        let size = "class_declaration Store > method_definition size";
        let item = "interface_declaration Item";
        let double = "variable_declarator double";
        assert_symbols(
            Language::TypeScript,
            &[
                ("import { readFile } from \"fs\";", ""),
                (
                    "export function count(): number {",
                    "function_declaration count",
                ),
                ("  return 1;", "function_declaration count"),
                ("}", "function_declaration count"),
                (
                    "function* ids() { yield 1; }",
                    "generator_function_declaration ids",
                ),
                ("export class Store {", store),
                ("  items = 0;", store),
                ("  /** Reserves one. */", store),
                ("  reserve(): void {", reserve),
                ("    [1].forEach((n) => {", reserve),
                ("      this.items -= n;", reserve),
                ("    });", reserve),
                ("  }", reserve),
                ("  release = () => {", release),
                ("    this.items += 1;", release),
                ("  };", release),
                ("  get size(): number { return this.items; }", size),
                ("  set size(n: number) { this.items = n; }", size),
                ("}", store),
                // A class decorator is part of its class; a method decorator is not part of its method.
                ("@sealed", "class_declaration Box"),
                ("class Box {", "class_declaration Box"),
                ("  @logged", "class_declaration Box"),
                (
                    "  open(): void {}",
                    "class_declaration Box > method_definition open",
                ),
                ("}", "class_declaration Box"),
                ("abstract class Shelf {", "abstract_class_declaration Shelf"),
                (
                    "  abstract put(): void;",
                    "abstract_class_declaration Shelf > abstract_method_signature put",
                ),
                ("}", "abstract_class_declaration Shelf"),
                ("interface Item {", item),
                ("  name: string;", item),
                (
                    "  label(): string;",
                    "interface_declaration Item > method_signature label",
                ),
                ("}", item),
                ("enum Kind { Item }", "enum_declaration Kind"),
                ("type Count = number;", "type_alias_declaration Count"),
                ("namespace Inventory {", "internal_module Inventory"),
                (
                    "  export const total = (a: number) => a;",
                    "internal_module Inventory > variable_declarator total",
                ),
                ("}", "internal_module Inventory"),
                ("const double = function (a: number) {", double),
                ("  return a * 2;", double),
                ("};", double),
                ("var ids2 = function* () {};", "variable_declarator ids2"),
                ("const limit = 9;", ""),
                ("describe(\"store\", () => {", ""),
                ("  it(\"reserves\", () => {});", ""),
                ("});", ""),
            ],
        );
    }

    #[test]
    fn python_symbols_are_functions_and_classes() {
        let store = "class_definition Store";
        let reserve = "class_definition Store > function_definition reserve";
        let release = "class_definition Store > function_definition release";
        assert_symbols(
            Language::Python,
            &[
                ("import os", ""),
                ("def count():", "function_definition count"),
                ("    return 1", "function_definition count"),
                ("class Store:", store),
                ("    limit = 9", store),
                ("    def reserve(self):", reserve),
                (
                    "        def inner():",
                    "class_definition Store > function_definition reserve > function_definition inner",
                ),
                (
                    "            pass",
                    "class_definition Store > function_definition reserve > function_definition inner",
                ),
                ("        take = lambda n: n", reserve),
                ("        return take(1)", reserve),
                ("", store),
                ("    @staticmethod", store),
                ("    async def release():", release),
                ("        return 2", release),
                ("LIMIT = 9", ""),
            ],
        );
    }

    #[test]
    fn enclosing_gives_the_symbols_of_lines() {
        let source = b"fn a() {\n}\n\nfn b() {\n}\n";
        let symbols = enclosing(Language::Rust, source, [1, 2, 3, 5]).unwrap();
        let names: Vec<String> = symbols.iter().map(Symbol::to_string).collect();
        assert_eq!(names, ["function_item a", "function_item b"]);
    }
}
