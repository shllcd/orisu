use crate::document::{Document, Encoding, Language};
use crate::features::{self, Ctx, SOURCE};
use crate::project::{self, Project};
use crate::settings::Settings;
use crossbeam_channel::TryRecvError;
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeConfiguration, DidChangeTextDocument, DidChangeWatchedFiles, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument,
    Notification as _, PublishDiagnostics,
};
use lsp_types::request::{
    CodeActionRequest, ColorPresentationRequest, Completion, DocumentColor, HoverRequest, RegisterCapability, Request as _,
    ResolveCompletionItem,
};
use lsp_types::{
    CodeActionKind, CodeActionOptions, CodeActionProviderCapability, ColorProviderCapability, CompletionOptions, Diagnostic,
    DiagnosticSeverity, DidChangeWatchedFilesRegistrationOptions, FileSystemWatcher, GlobPattern, HoverProviderCapability,
    InitializeParams, InitializeResult, Position, PositionEncodingKind, PublishDiagnosticsParams, Range, Registration, RegistrationParams,
    SaveOptions, ServerCapabilities, ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
    TextDocumentSyncSaveOptions, Uri,
};
use rustc_hash::FxHashMap;
use serde::Serialize;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::str::FromStr;

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

pub fn run() -> Result<()> {
    let (conn, io) = Connection::stdio();
    let (id, params) = conn.initialize_start()?;
    let params: InitializeParams = serde_json::from_value(params)?;
    let utf8 = params
        .capabilities
        .general
        .as_ref()
        .and_then(|g| g.position_encodings.as_ref())
        .is_some_and(|encodings| encodings.contains(&PositionEncodingKind::UTF8));
    let encoding = if utf8 { Encoding::Utf8 } else { Encoding::Utf16 };
    let result = InitializeResult {
        capabilities: capabilities(encoding),
        server_info: Some(ServerInfo { name: "orisu-lsp".to_owned(), version: Some(env!("CARGO_PKG_VERSION").to_owned()) }),
    };
    conn.initialize_finish(id, serde_json::to_value(result)?)?;
    let mut server = Server::new(conn, &params, encoding);
    server.reload();
    if params.capabilities.workspace.as_ref().and_then(|w| w.did_change_watched_files).is_some_and(|w| w.dynamic_registration == Some(true))
    {
        server.watch()?;
    }
    server.main_loop()?;
    drop(server);
    io.join()?;
    Ok(())
}

fn capabilities(encoding: Encoding) -> ServerCapabilities {
    ServerCapabilities {
        position_encoding: Some(if encoding == Encoding::Utf8 { PositionEncodingKind::UTF8 } else { PositionEncodingKind::UTF16 }),
        text_document_sync: Some(TextDocumentSyncCapability::Options(TextDocumentSyncOptions {
            open_close: Some(true),
            change: Some(TextDocumentSyncKind::FULL),
            save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions { include_text: Some(false) })),
            ..TextDocumentSyncOptions::default()
        })),
        completion_provider: Some(CompletionOptions {
            resolve_provider: Some(true),
            trigger_characters: Some(["\"", "'", "`", " ", "(", "[", "!", "/", "-", ":"].map(str::to_owned).to_vec()),
            ..CompletionOptions::default()
        }),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        code_action_provider: Some(CodeActionProviderCapability::Options(CodeActionOptions {
            code_action_kinds: Some(vec![CodeActionKind::QUICKFIX, CodeActionKind::REFACTOR_REWRITE]),
            ..CodeActionOptions::default()
        })),
        color_provider: Some(ColorProviderCapability::Simple(true)),
        ..ServerCapabilities::default()
    }
}

struct Server {
    conn: Connection,
    encoding: Encoding,
    roots: Vec<PathBuf>,
    settings: Settings,
    projects: Vec<Project>,
    /// Entry points that failed to compile, with the error.
    failures: Vec<(PathBuf, String)>,
    documents: FxHashMap<Uri, Document>,
    watching: bool,
    item_defaults: bool,
    /// Documents whose canonical-class hints wait until no message is pending.
    stale: Vec<Uri>,
    next_id: i32,
}

impl Server {
    fn new(conn: Connection, params: &InitializeParams, encoding: Encoding) -> Self {
        let mut roots: Vec<PathBuf> = params.workspace_folders.iter().flatten().filter_map(|folder| uri_path(&folder.uri)).collect();
        #[expect(deprecated, reason = "older clients only send `rootUri`")]
        let root_uri = params.root_uri.as_ref();
        if roots.is_empty() {
            roots.extend(root_uri.and_then(uri_path));
        }
        let settings = params.initialization_options.as_ref().map(Settings::from_value).unwrap_or_default();
        let item_defaults = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|t| t.completion.as_ref()?.completion_list.as_ref()?.item_defaults.as_ref())
            .is_some_and(|defaults| ["editRange", "data"].iter().all(|d| defaults.iter().any(|x| x == d)));
        Self {
            conn,
            encoding,
            roots,
            settings,
            projects: Vec::new(),
            failures: Vec::new(),
            documents: FxHashMap::default(),
            watching: false,
            item_defaults,
            stale: Vec::new(),
            next_id: 0,
        }
    }

    fn main_loop(&mut self) -> Result<()> {
        let receiver = self.conn.receiver.clone();
        loop {
            let message = match receiver.try_recv() {
                Ok(message) => message,
                Err(TryRecvError::Empty) => {
                    if let Some(uri) = self.stale.pop() {
                        self.publish_with(&uri, true);
                        continue;
                    }
                    match receiver.recv() {
                        Ok(message) => message,
                        Err(_) => return Ok(()),
                    }
                }
                Err(TryRecvError::Disconnected) => return Ok(()),
            };
            match message {
                Message::Request(request) => {
                    if self.conn.handle_shutdown(&request)? {
                        return Ok(());
                    }
                    self.on_request(request)?;
                }
                Message::Notification(notification) => self.on_notification(notification)?,
                Message::Response(_) => {}
            }
        }
    }

    fn watch(&mut self) -> Result<()> {
        let options = DidChangeWatchedFilesRegistrationOptions {
            watchers: vec![FileSystemWatcher { glob_pattern: GlobPattern::String("**/*.css".to_owned()), kind: None }],
        };
        let registration = Registration {
            id: "orisu-css".to_owned(),
            method: DidChangeWatchedFiles::METHOD.to_owned(),
            register_options: Some(serde_json::to_value(options)?),
        };
        self.next_id += 1;
        let params = RegistrationParams { registrations: vec![registration] };
        self.conn.sender.send(Request::new(RequestId::from(self.next_id), RegisterCapability::METHOD.to_owned(), params).into())?;
        self.watching = true;
        Ok(())
    }

    fn reload(&mut self) {
        let failed: Vec<PathBuf> = self.failures.drain(..).map(|(path, _)| path).collect();
        self.stale.clear();
        let mut previous: Vec<Project> = std::mem::take(&mut self.projects);
        let entries = match self.settings.config_file.as_slice() {
            [] => project::discover(&self.roots),
            files => {
                let root = self.roots.first().cloned().unwrap_or_default();
                files.iter().map(|file| root.join(file)).collect()
            }
        };
        for entry in entries {
            match Project::load(&entry) {
                Ok(project) => self.projects.push(project),
                Err(error) => {
                    eprintln!("orisu-lsp: {}: {error}", entry.display());
                    if let Some(at) = previous.iter().position(|p| p.entry == entry) {
                        self.projects.push(previous.swap_remove(at));
                    }
                    self.failures.push((entry, error));
                }
            }
        }
        self.projects.sort_by_key(|p| std::cmp::Reverse(p.dir.components().count()));
        eprintln!("orisu-lsp: {} project(s): {:?}", self.projects.len(), self.projects.iter().map(|p| &p.entry).collect::<Vec<_>>());
        for path in failed {
            if !self.failures.iter().any(|(p, _)| *p == path) {
                self.publish_raw(path_uri(&path), Vec::new(), None);
            }
        }
        for (path, error) in &self.failures {
            let diagnostic = Diagnostic {
                range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some(SOURCE.to_owned()),
                message: error.clone(),
                ..Diagnostic::default()
            };
            self.publish_raw(path_uri(path), vec![diagnostic], None);
        }
        let uris: Vec<Uri> = self.documents.keys().cloned().collect();
        for uri in uris {
            self.publish(&uri);
        }
    }

    /// Whether a change to the stylesheet at `path` can change a project.
    fn affects(&self, path: &Path) -> bool {
        self.projects.iter().any(|p| p.entry == path || p.dependencies.iter().any(|d| d == path))
            || self.failures.iter().any(|(entry, _)| entry == path)
            || std::fs::read_to_string(path).is_ok_and(|css| project::is_entry(&css))
    }

    fn project_for(&self, uri: &Uri) -> Option<&Project> {
        let path = uri_path(uri);
        path.and_then(|path| self.projects.iter().find(|p| path.starts_with(&p.dir))).or(self.projects.first())
    }

    fn ctx(&self, uri: &Uri) -> Option<Ctx<'_>> {
        Some(Ctx {
            project: self.project_for(uri)?,
            doc: self.documents.get(uri)?,
            settings: &self.settings,
            encoding: self.encoding,
            item_defaults: self.item_defaults,
        })
    }

    fn publish(&mut self, uri: &Uri) {
        let ready = self.project_for(uri).is_none_or(Project::canonical_ready);
        if !ready && !self.stale.contains(uri) {
            self.stale.push(uri.clone());
        }
        self.publish_with(uri, ready);
    }

    fn publish_with(&self, uri: &Uri, canonical: bool) {
        if self.failures.iter().any(|(path, _)| uri_path(uri).as_deref() == Some(path)) || !self.documents.contains_key(uri) {
            return;
        }
        let diagnostics = self.ctx(uri).map(|ctx| ctx.diagnostics(uri, canonical)).unwrap_or_default();
        self.publish_raw(uri.clone(), diagnostics, self.documents.get(uri).map(|d| d.version));
    }

    fn publish_raw(&self, uri: Uri, diagnostics: Vec<Diagnostic>, version: Option<i32>) {
        let params = PublishDiagnosticsParams { uri, diagnostics, version };
        let _ = self.conn.sender.send(Notification::new(PublishDiagnostics::METHOD.to_owned(), params).into());
    }

    fn on_request(&mut self, request: Request) -> Result<()> {
        let response = match request.method.as_str() {
            HoverRequest::METHOD => respond::<HoverRequest, _>(request, |params| {
                let at = params.text_document_position_params;
                let ctx = self.ctx(&at.text_document.uri)?;
                ctx.hover(ctx.doc.offset(at.position, self.encoding))
            }),
            Completion::METHOD => respond::<Completion, _>(request, |params| {
                let at = params.text_document_position;
                let ctx = self.ctx(&at.text_document.uri)?;
                ctx.completion(ctx.doc.offset(at.position, self.encoding))
            }),
            ResolveCompletionItem::METHOD => respond::<ResolveCompletionItem, _>(request, |item| {
                let entry = item.data.as_ref().and_then(|d| d.get("entry")).and_then(|e| e.as_str()).map(PathBuf::from);
                match self.projects.iter().find(|p| Some(&p.entry) == entry.as_ref()).or(self.projects.first()) {
                    Some(project) => features::resolve_completion(project, item),
                    None => item,
                }
            }),
            CodeActionRequest::METHOD => respond::<CodeActionRequest, _>(request, |params| {
                let uri = &params.text_document.uri;
                Some(self.ctx(uri)?.code_actions(uri, params.range, &params.context.diagnostics))
            }),
            DocumentColor::METHOD => respond::<DocumentColor, _>(request, |params| {
                self.ctx(&params.text_document.uri).map(|ctx| ctx.colors()).unwrap_or_default()
            }),
            ColorPresentationRequest::METHOD => {
                respond::<ColorPresentationRequest, Vec<lsp_types::ColorPresentation>>(request, |_| Vec::new())
            }
            _ => Response::new_err(request.id, ErrorCode::MethodNotFound as i32, format!("Unhandled method {}", request.method)),
        };
        self.conn.sender.send(response.into())?;
        Ok(())
    }

    fn on_notification(&mut self, notification: Notification) -> Result<()> {
        match notification.method.as_str() {
            DidOpenTextDocument::METHOD => {
                let params = parse::<DidOpenTextDocument>(notification)?;
                let item = params.text_document;
                let language = Language::detect(&item.language_id, item.uri.as_str());
                self.documents.insert(item.uri.clone(), Document::new(item.text, language, item.version));
                self.publish(&item.uri);
            }
            DidChangeTextDocument::METHOD => {
                let params = parse::<DidChangeTextDocument>(notification)?;
                let uri = params.text_document.uri;
                if let (Some(change), Some(doc)) = (params.content_changes.into_iter().last(), self.documents.get_mut(&uri)) {
                    *doc = Document::new(change.text, doc.language, params.text_document.version);
                }
                self.publish(&uri);
            }
            DidCloseTextDocument::METHOD => {
                let uri = parse::<DidCloseTextDocument>(notification)?.text_document.uri;
                self.documents.remove(&uri);
                self.stale.retain(|stale| *stale != uri);
                self.publish_raw(uri, Vec::new(), None);
            }
            DidSaveTextDocument::METHOD => {
                let uri = parse::<DidSaveTextDocument>(notification)?.text_document.uri;
                if !self.watching && uri_path(&uri).is_some_and(|path| self.affects(&path)) {
                    self.reload();
                }
            }
            DidChangeWatchedFiles::METHOD => {
                let changes = parse::<DidChangeWatchedFiles>(notification)?.changes;
                if changes.iter().filter_map(|change| uri_path(&change.uri)).any(|path| self.affects(&path)) {
                    self.reload();
                }
            }
            DidChangeConfiguration::METHOD => {
                let settings = parse::<DidChangeConfiguration>(notification)?.settings;
                if !settings.is_null() {
                    self.settings = Settings::from_value(&settings);
                    self.reload();
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn respond<R: lsp_types::request::Request, T: Serialize>(request: Request, handle: impl FnOnce(R::Params) -> T) -> Response {
    match serde_json::from_value::<R::Params>(request.params) {
        Ok(params) => Response::new_ok(request.id, handle(params)),
        Err(error) => Response::new_err(request.id, ErrorCode::InvalidParams as i32, error.to_string()),
    }
}

fn parse<N: lsp_types::notification::Notification>(notification: Notification) -> Result<N::Params> {
    Ok(serde_json::from_value(notification.params)?)
}

fn uri_path(uri: &Uri) -> Option<PathBuf> {
    let path = uri.as_str().strip_prefix("file://")?;
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let mut bytes = Vec::with_capacity(path.len());
    let mut i = 0;
    let raw = path.as_bytes();
    while i < raw.len() {
        if raw[i] == b'%'
            && let Some(byte) = raw.get(i + 1..i + 3).and_then(|hex| u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok())
        {
            bytes.push(byte);
            i += 3;
            continue;
        }
        bytes.push(raw[i]);
        i += 1;
    }
    let decoded = String::from_utf8(bytes).ok()?;
    let windows = decoded.len() > 2 && decoded.as_bytes()[2] == b':' && decoded.starts_with('/');
    Some(PathBuf::from(if windows { &decoded[1..] } else { &decoded }))
}

fn path_uri(path: &Path) -> Uri {
    let path = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from(if path.starts_with('/') { "file://" } else { "file:///" });
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    Uri::from_str(&out).unwrap_or_else(|_| Uri::from_str("file:///").expect("a valid URI"))
}
