use anyhow::Context;
use std::collections::HashMap;
use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::{Node, Parser, TreeCursor};

#[derive(Debug, Default)]
pub struct SymbolTable {
    pub functions: HashMap<String, FunctionSymbol>,
    // global_variables: HashMap<String, VariableSymbol>,
}
impl SymbolTable {
    pub fn from_source(source: &str) -> anyhow::Result<Self> {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_nesfab::language())?;

        let tree = parser
            .parse(source, None)
            .context("failed to parse source code")?;
        let root_node = tree.root_node();
        let mut cursor = root_node.walk();
        let mut symbol_table = SymbolTable::default();
        traverse_tree(source, &mut cursor, &mut symbol_table)?;
        Ok(symbol_table)
    }
    pub fn find_symbol(&self, node: &Node, name: &str) -> anyhow::Result<impl Symbol> {
        let parent = node.parent().context("faield to get parent")?;
        let symbol = match parent.kind() {
            "call" => self.functions.get(name).map(|s| s.to_owned()),
            _ => match parent.parent().context("failed to get parent")?.kind() {
                "function_definition" | "asm_function_definition" => {
                    self.functions.get(name).map(|s| s.to_owned())
                }
                _ => None,
            },
        };
        symbol.context("failed to find symbol: {name}")
    }
}

pub trait Symbol {
    fn from_node(source: &str, node: &Node) -> anyhow::Result<Self>
    where
        Self: Sized;
    fn range(&self) -> Range;
    fn description(&self) -> &str;
}

#[derive(Debug, Default, Clone)]
pub struct FunctionSymbol {
    pub range: Range,

    pub name: String,
    // arguments: Vec<VariableSymbol>,
    // return_type: TypeSymbol,
    // modifiers: Vec<ModifierSymbol>,
    // local_variables: Vec<VariableSymbol>,
    pub description: String,
}

fn collect_sibling_comment_nodes(node: Node) -> Vec<Node> {
    let mut comments = Vec::new();
    let mut pivot_line_number = node.start_position().row as isize;
    let mut pivot = Some(node);
    loop {
        if let Some(node) = pivot {
            if node.kind() == "comment"
                && (pivot_line_number - (node.end_position().row as isize) <= 1)
            {
                comments.push(node);
                pivot_line_number = node.start_position().row as isize;
            } else {
                break;
            }
            pivot = node.prev_sibling();
        } else {
            break;
        }
    }
    comments
}

impl Symbol for FunctionSymbol {
    fn from_node(source: &str, node: &Node) -> anyhow::Result<Self> {
        let signature = node.child_by_field_name("signature").context(format!(
            "failed to get signature node: {:?}",
            node.byte_range()
        ))?;
        let bytes = source.as_bytes();
        let name = signature
            .child_by_field_name("name")
            .context(format!("failed to get node: {:?}", node.byte_range()))
            .and_then(|node| {
                node.utf8_text(source.as_bytes())
                    .map_err(anyhow::Error::from)
            })?;
        let comments = node
            .prev_sibling()
            .map(|node| collect_sibling_comment_nodes(node))
            .map(|comments| {
                comments.iter().rfold(String::new(), |acc, x| {
                    acc + x.utf8_text(bytes).unwrap_or("") + "\n"
                })
            });
        let description = comments.unwrap_or("".to_string()) + signature.utf8_text(bytes)?;
        let node_range = node.range();
        let range = Range {
            start: Position::new(
                node_range.start_point.row as u32,
                node_range.start_point.column as u32,
            ),
            end: Position::new(
                node_range.end_point.row as u32,
                node_range.end_point.column as u32,
            ),
        };

        Ok(FunctionSymbol {
            name: name.to_string(),
            range,
            description,
        })
    }
    fn range(&self) -> Range {
        self.range.to_owned()
    }
    fn description(&self) -> &str {
        self.description.as_str()
    }
}

pub fn traverse_tree(
    source: &str,
    cursor: &mut TreeCursor,
    symbol_table: &mut SymbolTable,
) -> anyhow::Result<()> {
    loop {
        let node = cursor.node();
        if node.is_named() {
            match node.kind() {
                "function_definition" | "asm_function_definition" => {
                    let symbol = FunctionSymbol::from_node(source, &node)?;
                    symbol_table.functions.insert(symbol.name.clone(), symbol);
                }
                _ => {}
            }
        }
        if cursor.goto_first_child() {
            traverse_tree(source, cursor, symbol_table)?;
            cursor.goto_parent();
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    Ok(())
}
