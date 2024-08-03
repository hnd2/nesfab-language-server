use crate::{cfg::collect_cfg_map, symbol::*};
use anyhow::Context;
use dashmap::{DashMap, DashSet};
use rayon::prelude::*;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};
use tower_lsp::{jsonrpc, lsp_types::*, Client, LanguageServer};
use tree_sitter::{Parser, Point, Tree};

pub struct Backend {
    pub client: Client,
    pub source_map: DashMap<PathBuf, String>,
    pub tree_map: DashMap<PathBuf, Tree>,
    pub symbol_map: DashMap<PathBuf, SymbolTable>,
    pub cfg_map: DashMap<PathBuf, HashSet<PathBuf>>,
    pub workspace_dirs: DashSet<PathBuf>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_nesfab::language())
            .expect("failed to set language: nesfab");
        Self {
            client,
            source_map: DashMap::new(),
            tree_map: DashMap::new(),
            symbol_map: DashMap::new(),
            cfg_map: DashMap::new(),
            workspace_dirs: DashSet::new(),
        }
    }

    async fn on_change(&self, params: TextDocumentItem) -> anyhow::Result<()> {
        let file_path = params.uri.to_file_path().unwrap();
        let source = &params.text;
        self.source_map.insert(file_path.clone(), source.clone());

        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_nesfab::language())?;

        let tree = parser
            .parse(&source, None)
            .context("failed to parse source")?;
        self.tree_map.insert(file_path.clone(), tree.clone());

        let root_node = tree.root_node();
        let mut cursor = root_node.walk();
        let mut symbol_table = SymbolTable::default();
        traverse_tree(source, &mut cursor, &mut symbol_table)?;
        self.symbol_map.insert(file_path, symbol_table);

        Ok(())
    }

    async fn on_change_workspace_folders(
        &self,
        event: WorkspaceFoldersChangeEvent,
    ) -> anyhow::Result<()> {
        let to_path_buf_set = |workspace_folders: &[WorkspaceFolder]| -> HashSet<PathBuf> {
            workspace_folders
                .iter()
                .map(|dir| dir.uri.to_file_path())
                .filter_map(|file_path| file_path.ok())
                .collect::<HashSet<_>>()
        };
        let cfg_files = self
            .cfg_map
            .iter()
            .map(|entry| entry.pair().0.to_owned())
            .collect::<HashSet<PathBuf>>()
            .difference(&to_path_buf_set(&event.removed))
            .cloned()
            .collect::<HashSet<_>>()
            .union(&to_path_buf_set(&event.added))
            .cloned()
            .filter(|path| !self.cfg_map.contains_key(path))
            .collect::<Vec<_>>();

        // reconstruct cfg_map
        let cfg_map = collect_cfg_map(&cfg_files)?;
        self.cfg_map.clear();
        for (key, value) in cfg_map.clone() {
            self.cfg_map.insert(key, value);
        }

        // reconstruct symbol_map
        let symbol_map = cfg_map
            .par_iter()
            .flat_map(|(_, files)| files)
            .cloned()
            .filter(|file| !self.symbol_map.contains_key(file))
            .collect::<HashSet<_>>()
            .par_iter()
            .filter_map(|file| {
                if let Ok(source) = std::fs::read_to_string(file) {
                    SymbolTable::from_source(&source)
                        .map(|symbol_table| (file.to_owned(), symbol_table))
                        .ok()
                } else {
                    None
                }
            })
            .collect::<HashMap<_, _>>();
        for (key, value) in symbol_map {
            self.client
                .log_message(MessageType::INFO, format!("symbol cached: {key:?}"))
                .await;
            self.symbol_map.insert(key, value);
        }

        Ok(())
    }

    fn hover(&self, file_path: &Path, point: &Point) -> anyhow::Result<Option<HoverContents>> {
        match self.find_symbol(file_path, point) {
            Ok(Some((file_path, symbol))) => {
                let marked_string = MarkedString::LanguageString(LanguageString {
                    language: "nesfab".to_string(),
                    value: symbol.description().to_string(),
                });
                let file_path = self.get_relative_path(&file_path).unwrap_or(file_path);
                let path_marked_string =
                    MarkedString::from_markdown(file_path.to_string_lossy().into());

                Ok(Some(HoverContents::Array(vec![
                    path_marked_string,
                    marked_string,
                ])))
            }
            Err(e) => Err(e),
            _ => Ok(None),
        }
    }

    fn goto_definition(
        &self,
        file_path: &Path,
        point: &Point,
    ) -> anyhow::Result<Option<(Url, Range)>> {
        match self.find_symbol(file_path, point) {
            Ok(Some((file_path, symbol))) => {
                let url = Url::from_file_path(file_path).unwrap();
                let range = symbol.range();
                Ok(Some((url, range.into())))
            }
            Err(e) => Err(e),
            _ => Ok(None),
        }
    }

    fn get_relative_path(&self, path: &Path) -> Option<PathBuf> {
        self.workspace_dirs
            .iter()
            .find_map(|file_path| path.strip_prefix(file_path.to_owned()).ok())
            .map(|path| path.to_path_buf())
    }

    fn find_symbol(
        &self,
        file_path: &Path,
        point: &Point,
    ) -> anyhow::Result<Option<(PathBuf, impl Symbol)>> {
        let source = self
            .source_map
            .get(file_path)
            .context(format!("failed to get source file: {file_path:?}"))?;
        let tree = self
            .tree_map
            .get(file_path)
            .context(format!("failed to get tree file: {file_path:?}"))?;
        let node = tree
            .root_node()
            .descendant_for_point_range(*point, *point)
            .context(format!("failed to get node file: {file_path:?}"))?;

        if node.kind() == "identifier" {
            let name = node.utf8_text(source.as_bytes())?;
            let pair = self
                .symbol_map
                .get(file_path)
                .and_then(|symbols| {
                    symbols
                        .find_symbol(&node, &name)
                        .map(|symbol| (file_path.to_owned(), symbol))
                        .ok()
                })
                .or(self
                    .symbol_map
                    .iter()
                    .filter(|entry| entry.key() != &file_path)
                    .find_map(|entry| {
                        let (path, symbols) = entry.pair();
                        symbols
                            .find_symbol(&node, &name)
                            .map(|symbol| (path.to_owned(), symbol))
                            .ok()
                    }));
            return Ok(pair);
        } else {
            return Ok(None);
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        let workspace_dirs = params
            .workspace_folders
            .iter()
            .flat_map(|workspace_folder| workspace_folder)
            .filter_map(|workspace_folder| workspace_folder.uri.to_file_path().ok())
            .collect::<HashSet<_>>();
        for workspace_dir in workspace_dirs.into_iter() {
            self.workspace_dirs.insert(workspace_dir);
        }

        Ok(InitializeResult {
            server_info: None,
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    file_operations: None,
                }),
                definition_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions::default()),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "initialized.")
            .await;

        let added = self
            .workspace_dirs
            .iter()
            .filter_map(|dir| {
                let uri = Url::from_file_path(dir.to_owned());
                let name = dir.file_name();
                match (uri, name) {
                    (Ok(uri), Some(name)) => Some(WorkspaceFolder {
                        uri,
                        name: name.to_string_lossy().to_string(),
                    }),
                    _ => None,
                }
            })
            .collect::<Vec<_>>();
        self.did_change_workspace_folders(DidChangeWorkspaceFoldersParams {
            event: WorkspaceFoldersChangeEvent {
                added,
                removed: vec![],
            },
        })
        .await;
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        self.client
            .log_message(MessageType::INFO, "shutdown.")
            .await;
        Ok(())
    }
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.client.log_message(MessageType::INFO, "did open").await;

        if let Err(err) = self.on_change(params.text_document.clone()).await {
            self.client
                .log_message(MessageType::ERROR, format!("{:?}", err))
                .await;
        }
    }
    async fn did_change(&self, mut params: DidChangeTextDocumentParams) {
        self.client
            .log_message(MessageType::INFO, "did change")
            .await;

        if let Err(err) = self
            .on_change(TextDocumentItem {
                uri: params.text_document.uri,
                text: std::mem::take(&mut params.content_changes[0].text),
                version: params.text_document.version,
                language_id: String::new(),
            })
            .await
        {
            self.client
                .log_message(MessageType::ERROR, format!("{:?}", err))
                .await;
        }
    }
    async fn did_save(&self, _: DidSaveTextDocumentParams) {
        self.client.log_message(MessageType::INFO, "did save").await;
    }
    async fn did_close(&self, _: DidCloseTextDocumentParams) {
        self.client
            .log_message(MessageType::INFO, "did close")
            .await;
    }
    async fn did_change_configuration(&self, _: DidChangeConfigurationParams) {
        self.client
            .log_message(MessageType::INFO, "did change configuration")
            .await;
        let a = self.client.workspace_folders().await;
        self.client
            .log_message(MessageType::INFO, format!("nannkakita: {:?}", a))
            .await;
        self.client
            .log_message(MessageType::INFO, "did change configuration")
            .await;
    }
    async fn did_change_workspace_folders(&self, params: DidChangeWorkspaceFoldersParams) {
        self.client
            .log_message(MessageType::INFO, "did change workspace folders")
            .await;

        if let Err(e) = self.on_change_workspace_folders(params.event.clone()).await {
            self.client
                .log_message(MessageType::ERROR, format!("error: {e}"))
                .await;
        }
    }
    async fn did_change_watched_files(&self, _: DidChangeWatchedFilesParams) {
        self.client
            .log_message(MessageType::INFO, "did change watched files")
            .await;
    }

    async fn completion(&self, _: CompletionParams) -> jsonrpc::Result<Option<CompletionResponse>> {
        Ok(Some(CompletionResponse::Array(vec![
            CompletionItem::new_simple("Hello".to_string(), "Some detail".to_string()),
            CompletionItem::new_simple("Bye".to_string(), "More detail".to_string()),
        ])))
    }
    async fn hover(&self, params: HoverParams) -> jsonrpc::Result<Option<Hover>> {
        let file_path = match params
            .text_document_position_params
            .text_document
            .uri
            .to_file_path()
        {
            Ok(ok) => ok,
            Err(e) => {
                self.client
                    .log_message(MessageType::ERROR, format!("error: {e:?}"))
                    .await;
                return Err(jsonrpc::Error::internal_error());
            }
        };
        let position = params.text_document_position_params.position;
        let point = Point::new(position.line as usize, position.character as usize);
        match self.hover(&file_path, &point) {
            Ok(Some(contents)) => Ok(Some(Hover {
                contents,
                range: None,
            })),
            Err(e) => {
                self.client
                    .log_message(MessageType::ERROR, format!("error: {e:?}"))
                    .await;
                Err(jsonrpc::Error::internal_error())
            }
            _ => Ok(None),
        }
    }
    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> jsonrpc::Result<Option<GotoDefinitionResponse>> {
        self.client
            .log_message(MessageType::INFO, format!("goto definition"))
            .await;

        let file_path = match params
            .text_document_position_params
            .text_document
            .uri
            .to_file_path()
        {
            Ok(ok) => ok,
            Err(e) => {
                self.client
                    .log_message(MessageType::ERROR, format!("error: {e:?}"))
                    .await;
                return Err(jsonrpc::Error::internal_error());
            }
        };
        let position = params.text_document_position_params.position;
        let point = Point::new(position.line as usize, position.character as usize);
        match self.goto_definition(&file_path, &point) {
            Ok(Some((url, range))) => Ok(Some(GotoDefinitionResponse::Scalar(Location::new(
                url, range,
            )))),
            _ => Ok(None),
        }
    }
}
