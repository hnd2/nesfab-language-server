use anyhow::{bail, Context};
use tree_sitter::{Parser, TreeCursor};

#[derive(Debug)]
struct Symbol {
    name: String,
    range: std::ops::Range<usize>,
}

fn main() -> anyhow::Result<()> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_nesfab::language())?;

    let source_code = r#"
        vars
            // Player variables:
            SS px = 128
            SS py = 120

            // Follower variables:
            SSF fx = 128.0
            SSF fy = 120.0

        mode main()
        : nmi game_nmi
            palette = example_palette
            ppu_upload_palette()
            {PPUCTRL}(PPUCTRL_NMI_ON)

            while true
                nmi
                update_pads()
                move_player()
                move_follower()
                update_sprites()

        fn move_player()
            ct Int SPEED = 2

            if pads[0].held & BUTTON_LEFT
                px -= SPEED
            else if pads[0].held & BUTTON_RIGHT
                px += SPEED

            if pads[0].held & BUTTON_UP
                py -= SPEED
            else if pads[0].held & BUTTON_DOWN
                py += SPEED

        fn move_follower()
            U dir = point_dir(SS(fx), SS(fy), px, py)
            fx += cos(dir)
            fy += sin(dir)

        fn update_sprites()
            // Our stack index into OAM:
            U o = 0

            // Player:
            if px.b == 0 && py.b == 0
                set_oam(o, px.a, py.a - 1, $00, 0)
                o += 4

            // Follower:
            if fx.b == 0 && fy.b == 0
                set_oam(o, fx.a, fy.a - 1, $00, 1)
                o += 4

            // Clear the remainder of OAM
            hide_oam(o)

        nmi game_nmi()
            // Update OAM and poll the pads:
            ppu_upload_oam_poll_pads(0)

            // Turn on rendering:
            {PPUMASK}(PPUMASK_SPR_ON | PPUMASK_NO_CLIP)

            // Reset the scroll
            ppu_reset_scroll(0, 0)

        chrrom
            U[16]($FF)
    "#;
    let tree = parser
        .parse(source_code, None)
        .context("failed to parse source code")?;
    let root_node = tree.root_node();

    // println!("Root node: {}", root_node.to_sexp());

    let mut symbols = Vec::new();
    let mut cursor = root_node.walk();
    traverse_tree(&mut cursor, source_code, &mut symbols)?;
    for symbol in &symbols {
        println!("Symbol: {}", symbol.name);
    }

    Ok(())
}

fn traverse_tree(
    cursor: &mut TreeCursor,
    source_code: &str,
    symbols: &mut Vec<Symbol>,
) -> anyhow::Result<()> {
    loop {
        let node = cursor.node();
        if node.is_named() {
            if node.kind() == "function_definition" {
                let name = node
                    .child_by_field_name("name")
                    .and_then(|node| node.utf8_text(source_code.as_bytes()).ok());
                if let Some(name) = name {
                    let symbol = Symbol {
                        name: name.to_string(),
                        range: node.byte_range(),
                    };
                    symbols.push(symbol);
                } else {
                    println!(
                        "failed to get function name: {:?}",
                        node.utf8_text(source_code.as_bytes())
                    );
                }
            }
        }
        if cursor.goto_first_child() {
            traverse_tree(cursor, source_code, symbols)?;
            cursor.goto_parent();
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    Ok(())
}
