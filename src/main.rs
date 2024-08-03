use std::sync::Mutex;

use anyhow::{bail, Context};
use nesfab_language_server::symbol::*;
use tower_lsp::{
    jsonrpc,
    lsp_types::{
        CompletionItem, CompletionOptions, CompletionParams, CompletionResponse,
        DidOpenTextDocumentParams, Hover, HoverContents, HoverParams, HoverProviderCapability,
        InitializeParams, InitializeResult, InitializedParams, LanguageString, MarkedString,
        MessageType, Position, ServerCapabilities,
    },
    Client, LanguageServer, LspService, Server,
};
use tree_sitter::{Node, Parser, Point, Tree};

struct Backend {
    client: Client,
    parser: Mutex<Parser>,
    symbol_table: SymbolTable,
}
impl Backend {
    fn new(client: Client) -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_nesfab::language())
            .expect("failed to set language: nesfab");
        Self {
            client,
            parser: Mutex::new(parser),
            symbol_table: SymbolTable::default(),
        }
    }
}

trait ResultExt<T> {
    fn to_jsonrpc_result(self) -> std::result::Result<T, jsonrpc::Error>;
}
impl<T, E> ResultExt<T> for anyhow::Result<T, E>
where
    E: std::fmt::Debug,
{
    fn to_jsonrpc_result(self) -> std::result::Result<T, jsonrpc::Error> {
        match self {
            Ok(ok) => Ok(ok),
            Err(e) => Err(jsonrpc::Error {
                code: jsonrpc::ErrorCode::InternalError,
                message: std::borrow::Cow::Owned(format!("{:?}", e)),
                data: None,
            }),
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        params.root_uri;
        Ok(InitializeResult {
            server_info: None,
            capabilities: ServerCapabilities {
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions::default()),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "server initialized.")
            .await;
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        Ok(())
    }
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        /*
        let Ok(mut symbol_table) = self.symbol_table.lock() else {
            self.client
                .log_message(MessageType::ERROR, "failed to lock symbol table")
                .await;
            return;
        };

        let path = params.text_document.uri.to_file_path().unwrap();
        let source = &params.text_document.text;

        let mut parser = self.parser.lock().unwrap();
        let tree = parser.parse(&source, None).unwrap();
        let root_node = tree.root_node();

        let mut cursor = root_node.walk();
        let _ = traverse_tree(&path, &source, &mut cursor, &mut symbol_table);
        */
    }

    async fn completion(&self, _: CompletionParams) -> jsonrpc::Result<Option<CompletionResponse>> {
        Ok(Some(CompletionResponse::Array(vec![
            CompletionItem::new_simple("Hello".to_string(), "Some detail".to_string()),
            CompletionItem::new_simple("Bye".to_string(), "More detail".to_string()),
        ])))
    }
    async fn hover(&self, params: HoverParams) -> jsonrpc::Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let source = std::fs::read_to_string(uri.path()).to_jsonrpc_result()?;

        let mut parser = self.parser.lock().to_jsonrpc_result()?;
        parser
            .set_language(&tree_sitter_nesfab::language())
            .expect("failed to set language: nesfab");

        let tree = parser
            .parse(&source, None)
            .ok_or(jsonrpc::Error::parse_error())?;

        let path = uri.to_file_path().unwrap();
        let root_node = tree.root_node();
        let mut cursor = root_node.walk();

        let mut symbol_table = SymbolTable::default();
        let _ = traverse_tree(&path, &source, &mut cursor, &mut symbol_table);

        let position = params.text_document_position_params.position;
        let point = Point::new(position.line as usize, position.character as usize);
        let node = tree
            .root_node()
            .descendant_for_point_range(point, point)
            .ok_or(jsonrpc::Error::internal_error())?;

        if node.kind() == "identifier" {
            if let Some(symbol) = find_symbol(&symbol_table, source.as_str(), &node) {
                let marked_string = MarkedString::LanguageString(LanguageString {
                    language: "nesfab".to_string(),
                    value: symbol.description().to_string(),
                });
                return Ok(Some(Hover {
                    contents: HoverContents::Scalar(marked_string),
                    range: None,
                }));
            }
        }

        Ok(Some(Hover {
            contents: HoverContents::Scalar(MarkedString::String("bbaz".to_string())),
            range: None,
        }))
    }
}
fn find_symbol(symbol_table: &SymbolTable, source: &str, node: &Node) -> Option<impl Symbol> {
    let Some(parent) = node.parent().and_then(|node| node.parent()) else {
        return None;
    };

    match parent.kind() {
        "function_definition" => {
            let name = node.utf8_text(source.as_bytes()).unwrap();
            symbol_table.functions.get(name).map(|s| s.clone())
        }
        _ => None,
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend::new(client));
    Server::new(stdin, stdout, socket).serve(service).await;
}
