use std::{
    collections::HashMap,
    ops::Range,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context};
use nesfab_language_server::symbol::*;
use tree_sitter::{Node, Parser, TreeCursor};

fn main() -> anyhow::Result<()> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_nesfab::language())?;

    let source_code = include_str!("main.fab");
    let tree = parser
        .parse(source_code, None)
        .context("failed to parse source code")?;
    let root_node = tree.root_node();

    println!("Root node: {}", root_node.to_sexp());

    let mut cursor = root_node.walk();

    let mut symbol_table = SymbolTable::default();
    traverse_tree(
        Path::new("main.fab"),
        source_code,
        &mut cursor,
        &mut symbol_table,
    )?;

    for (name, symbol) in symbol_table.functions.iter() {
        println!("{}: {:?}", name, symbol);
    }

    // let line = 11;
    // let character = 13;

    Ok(())
}
