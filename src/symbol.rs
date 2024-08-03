use anyhow::Context;
use std::{
    collections::HashMap,
    ops::Range,
    path::{Path, PathBuf},
};
use tree_sitter::{Node, TreeCursor};

#[derive(Debug, Default)]
pub struct SymbolTable {
    pub functions: HashMap<String, FunctionSymbol>,
    // global_variables: HashMap<String, VariableSymbol>,
}

pub trait Symbol {
    fn from_node(uri: &Path, source: &str, node: &Node) -> anyhow::Result<Self>
    where
        Self: Sized;
    fn uri(&self) -> &Path;
    fn range(&self) -> Range<usize>;
    fn description(&self) -> &str;
}

fn traverse_error_message(uri: &Path, node: &Node) -> String {
    format!("{}:{:?}", uri.display(), node.byte_range())
}
#[derive(Debug, Default, Clone)]
pub struct FunctionSymbol {
    pub uri: PathBuf,
    pub range: Range<usize>,

    pub name: String,
    // arguments: Vec<VariableSymbol>,
    // return_type: TypeSymbol,
    // modifiers: Vec<ModifierSymbol>,
    // local_variables: Vec<VariableSymbol>,
    pub description: String,
}
impl Symbol for FunctionSymbol {
    fn from_node(uri: &Path, source: &str, node: &Node) -> anyhow::Result<Self> {
        let signature = node.child_by_field_name("signature").context(format!(
            "{:?}: failed to get signature node",
            traverse_error_message(uri, node)
        ))?;
        let name = signature
            .child_by_field_name("name")
            .context(format!(
                "{:?}: failed to get node",
                traverse_error_message(uri, node)
            ))
            .and_then(|node| {
                node.utf8_text(source.as_bytes())
                    .map_err(anyhow::Error::from)
            })?;
        Ok(FunctionSymbol {
            uri: uri.to_path_buf(),
            name: name.to_string(),
            range: node.byte_range(),
            description: signature.utf8_text(source.as_bytes())?.to_string(),
        })
    }
    fn uri(&self) -> &Path {
        &self.uri
    }
    fn range(&self) -> Range<usize> {
        self.range.to_owned()
    }
    fn description(&self) -> &str {
        self.description.as_str()
    }
}

pub fn traverse_tree(
    uri: &Path,
    source: &str,
    cursor: &mut TreeCursor,
    symbol_table: &mut SymbolTable,
) -> anyhow::Result<()> {
    loop {
        let node = cursor.node();
        if node.is_named() {
            if node.kind() == "function_definition" {
                let symbol = FunctionSymbol::from_node(&uri, source, &node)?;
                symbol_table.functions.insert(symbol.name.clone(), symbol);
            }
        }
        if cursor.goto_first_child() {
            traverse_tree(uri, source, cursor, symbol_table)?;
            cursor.goto_parent();
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    Ok(())
}
