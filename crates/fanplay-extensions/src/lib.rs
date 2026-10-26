// SPDX-License-Identifier: GPL-2.0-or-later
//! Rhai extension host.
//!
//! A plugin is a package without process-wide privileges. The
//! host owns package loading, operation registration, request queues and
//! lifecycle. Rhai only orchestrates those capabilities and produces a
//! declarative UI tree. Packages that declare `ruby.inject` and
//! `ruby.unload` also get `ruby_inject(name, source)`,
//! `ruby_inject_source(name, apply, revert)` and `ruby_unload(handle)`. Those
//! functions enqueue asynchronous, reversible runtime-patch requests; they
//! never evaluate Ruby on the UI thread. Plugins with `ui.webview` can also
//! enqueue `webview_open(view_id, title, html)` and `webview_close(view_id)`.

use rhai::{Array, Dynamic, Engine, ImmutableString, Map, Scope, AST};
use fanplay_api::OperationRegistry;
use fanplay_extension_api::{
    ExtensionEvent, ExtensionLimits, ExtensionManifest, ManifestError, OperationDeclaration,
    OperationError, OperationRequest, OperationResponse, PanelDeclaration, RouteAlias,
    UiDescription, UiEvent, UiNode,
};
use serde_json::{Map as JsonMap, Value};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// A package loaded from an installed directory or an unpacked browser blob.
#[derive(Clone, Debug)]
pub struct ExtensionPackage {
    pub root: PathBuf,
    pub manifest: ExtensionManifest,
    pub main_source: String,
    pub ui_source: Option<String>,
    pub ruby_source: Option<String>,
    /// Named raw text blocks removed from the Rhai source before compilation.
    /// A script reads one explicitly with `text_block("name")`.
    pub text_sources: BTreeMap<String, String>,
}

impl ExtensionPackage {
    pub fn load(root: impl AsRef<Path>) -> Result<Self, ExtensionError> {
        let root = root.as_ref().to_path_buf();
        if root.is_file() && root.extension().and_then(|value| value.to_str()) == Some("rhai") {
            return Self::from_rhai_file(&root);
        }
        Err(ExtensionError::Package(format!(
            "{} is not a .rhai plugin file",
            root.display()
        )))
    }

    /// Reads a self-contained Rhai extension from disk.
    pub fn from_rhai_file(path: impl AsRef<Path>) -> Result<Self, ExtensionError> {
        let path = path.as_ref();
        let source = fs::read_to_string(path)
            .map_err(|error| ExtensionError::Package(format!("{}: {error}", path.display())))?;
        let id = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| ExtensionError::Package("Rhai plugin has no filename ID".into()))?;
        Self::from_rhai(id, source)
    }

    /// Creates a self-contained plugin from pasted or imported Rhai text.
    /// The source may include `// fanplay:` metadata comments, but no manifest is
    /// required. `id` is used when the source does not declare one. A plugin
    /// may contain named raw text blocks in the form
    /// `/* fanplay:text name-begin ... fanplay:text name-end */`; the script inserts
    /// one explicitly with `text_block("name")`.
    pub fn from_rhai(id: impl Into<String>, source: String) -> Result<Self, ExtensionError> {
        let (rhai_source, text_sources) = extract_text_blocks(&source)?;
        let mut package = Self::from_rhai_with_ruby(id, rhai_source, None)?;
        package.text_sources = text_sources;
        Ok(package)
    }

    /// Creates a self-contained Rhai plugin while keeping optional Ruby
    /// source in memory. This is useful for a single Rhai entry file that
    /// embeds or otherwise supplies its Ruby implementation without TOML.
    pub fn from_rhai_with_ruby(
        id: impl Into<String>,
        source: String,
        ruby_source: Option<String>,
    ) -> Result<Self, ExtensionError> {
        let fallback_id = id.into();
        let manifest = rhai_manifest(&fallback_id, &source)?;
        manifest.validate().map_err(ExtensionError::Manifest)?;
        Ok(Self {
            root: PathBuf::from(format!("rhai://{}", manifest.id)),
            manifest,
            main_source: source,
            ui_source: None,
            ruby_source,
            text_sources: BTreeMap::new(),
        })
    }
}

fn extract_text_blocks(source: &str) -> Result<(String, BTreeMap<String, String>), ExtensionError> {
    const PREFIX: &str = "/* fanplay:text ";
    let mut rhai = String::with_capacity(source.len());
    let mut blocks = BTreeMap::new();
    let mut cursor = 0;
    while let Some(relative_begin) = source[cursor..].find(PREFIX) {
        let begin = cursor + relative_begin;
        rhai.push_str(&source[cursor..begin]);
        let header_start = begin + PREFIX.len();
        let header_end = source[header_start..]
            .find('\n')
            .map(|offset| header_start + offset)
            .ok_or_else(|| {
                ExtensionError::Package("embedded text block is missing a header line".into())
            })?;
        let name = source[header_start..header_end]
            .trim()
            .strip_suffix("-begin")
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                ExtensionError::Package(
                    "embedded text block must start with `/* fanplay:text name-begin`".into(),
                )
            })?;
        let end_marker = format!("fanplay:text {name}-end */");
        let content_start = header_end + 1;
        let content_end = source[content_start..]
            .find(&end_marker)
            .map(|offset| content_start + offset)
            .ok_or_else(|| {
                ExtensionError::Package(format!(
                    "embedded text block `{name}` is missing `{end_marker}`"
                ))
            })?;
        if blocks.contains_key(name) {
            return Err(ExtensionError::Package(format!(
                "embedded text block `{name}` is declared more than once"
            )));
        }
        let content = source[content_start..content_end]
            .strip_prefix('\n')
            .unwrap_or(&source[content_start..content_end])
            .to_owned();
        blocks.insert(name.to_owned(), content);
        cursor = content_end + end_marker.len();
    }
    rhai.push_str(&source[cursor..]);
    Ok((rhai, blocks))
}

fn rhai_manifest(fallback_id: &str, source: &str) -> Result<ExtensionManifest, ExtensionError> {
    let mut manifest = ExtensionManifest {
        api_version: fanplay_extension_api::EXTENSION_API_VERSION,
        id: fallback_id.to_owned(),
        name: fallback_id.replace(['-', '_'], " "),
        version: "0.1.0".into(),
        ..Default::default()
    };
    let mut route_metadata = Vec::new();
    let mut operation_capability_metadata = Vec::new();
    let mut operation_mutation_metadata = Vec::new();
    for line in source.lines() {
        let Some(metadata) = line.trim().strip_prefix("// fanplay:") else {
            continue;
        };
        let Some((key, value)) = metadata.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"');
        match key {
            "id" => manifest.id = value.to_owned(),
            "name" => manifest.name = value.to_owned(),
            "version" => manifest.version = value.to_owned(),
            "capabilities" => {
                manifest.capabilities = value
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            "operation" => manifest.operations.push(OperationDeclaration {
                id: value.to_owned(),
                ..Default::default()
            }),
            "operation-capabilities" => operation_capability_metadata.push(value.to_owned()),
            "operation-mutates" => operation_mutation_metadata.push(value.to_owned()),
            "route" => route_metadata.push(value.to_owned()),
            "panel" => {
                let mut parts = value.splitn(2, ',').map(str::trim);
                if let Some(id) = parts.next().filter(|id| !id.is_empty()) {
                    manifest.panels.push(PanelDeclaration {
                        id: id.to_owned(),
                        title: parts.next().unwrap_or(id).to_owned(),
                        ..Default::default()
                    });
                }
            }
            _ => {}
        }
    }
    for route in route_metadata {
        let mut parts = route.split_whitespace();
        let (Some(method), Some(path), Some(operation)) =
            (parts.next(), parts.next(), parts.next())
        else {
            return Err(ExtensionError::Package(format!(
                "invalid // fanplay:route metadata: {route}"
            )));
        };
        let arguments = parts
            .next()
            .unwrap_or_default()
            .split(',')
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect();
        let declaration = manifest
            .operations
            .iter_mut()
            .find(|candidate| candidate.id == operation)
            .ok_or_else(|| {
                ExtensionError::Package(format!(
                    "// fanplay:route references undeclared operation: {operation}"
                ))
            })?;
        declaration.route_aliases.push(RouteAlias {
            method: method.to_owned(),
            path: path.to_owned(),
            arguments,
        });
    }
    for metadata in operation_capability_metadata {
        let mut parts = metadata.splitn(2, char::is_whitespace).map(str::trim);
        let operation = parts.next().unwrap_or_default();
        let capabilities = parts
            .next()
            .unwrap_or_default()
            .split(',')
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let declaration = manifest
            .operations
            .iter_mut()
            .find(|candidate| candidate.id == operation)
            .ok_or_else(|| {
                ExtensionError::Package(format!(
                    "// fanplay:operation-capabilities references undeclared operation: {operation}"
                ))
            })?;
        declaration.required_capabilities = capabilities;
    }
    for operation in operation_mutation_metadata {
        let declaration = manifest
            .operations
            .iter_mut()
            .find(|candidate| candidate.id == operation)
            .ok_or_else(|| {
                ExtensionError::Package(format!(
                    "// fanplay:operation-mutates references undeclared operation: {operation}"
                ))
            })?;
        declaration.mutates_state = true;
    }
    Ok(manifest)
}

/// Native backends implement this when they want to hand requests directly to
/// a core. The default host keeps an internal bounded queue, which is enough
/// for desktop, browser and headless adapters alike.
pub trait RequestSink {
    fn submit(&mut self, request: OperationRequest) -> Result<(), OperationError>;
}

#[derive(Default)]
struct QueuedRequests {
    generation: u64,
    prefix: u64,
    next_id: u64,
    max_requests: usize,
    queue: VecDeque<OperationRequest>,
}

#[derive(Default)]
struct ScriptState {
    values: Map,
    requests: QueuedRequests,
    last_error: Option<String>,
}

/// An active script and its package-owned state.
pub struct ExtensionInstance {
    pub manifest: ExtensionManifest,
    engine: Engine,
    scope: Scope<'static>,
    ast: AST,
    ui_ast: Option<AST>,
    state: Arc<Mutex<ScriptState>>,
    limits: ExtensionLimits,
    ui: Option<UiDescription>,
}

impl fmt::Debug for ExtensionInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtensionInstance")
            .field("id", &self.manifest.id)
            .field("version", &self.manifest.version)
            .field("ui", &self.ui)
            .finish_non_exhaustive()
    }
}

impl ExtensionInstance {
    pub fn load(package: ExtensionPackage) -> Result<Self, ExtensionError> {
        Self::load_with_limits(package, ExtensionLimits::default())
    }

    pub fn load_with_limits(
        package: ExtensionPackage,
        limits: ExtensionLimits,
    ) -> Result<Self, ExtensionError> {
        let mut engine = Engine::new();
        engine.set_max_operations(limits.max_operations);
        engine.set_max_call_levels(limits.max_call_levels);
        engine.set_max_string_size(limits.max_string_size);
        engine.set_max_array_size(limits.max_array_size);
        engine.set_max_map_size(limits.max_map_size);

        let state = Arc::new(Mutex::new(ScriptState {
            requests: QueuedRequests {
                max_requests: limits.max_requests,
                ..Default::default()
            },
            ..Default::default()
        }));
        let ruby_inject = package
            .manifest
            .capabilities
            .iter()
            .any(|capability| capability == "ruby.inject");
        let ruby_unload = package
            .manifest
            .capabilities
            .iter()
            .any(|capability| capability == "ruby.unload");
        let allow_webview = package
            .manifest
            .capabilities
            .iter()
            .any(|capability| capability == "ui.webview");
        register_host_functions(
            &mut engine,
            &state,
            &package.manifest.id,
            ruby_inject,
            ruby_unload,
            allow_webview,
            package.ruby_source.clone(),
            package.text_sources.clone(),
        );
        let ast = engine
            .compile(&package.main_source)
            .map_err(|error| ExtensionError::Script(error.to_string()))?;
        let ui_ast = package
            .ui_source
            .as_deref()
            .map(|source| {
                engine
                    .compile(source)
                    .map_err(|error| ExtensionError::Script(error.to_string()))
            })
            .transpose()?;
        Ok(Self {
            manifest: package.manifest,
            engine,
            scope: Scope::new(),
            ast,
            ui_ast,
            state,
            limits,
            ui: None,
        })
    }

    pub fn generation(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .requests
            .generation
    }

    pub fn set_generation(&mut self, generation: u64) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.requests.generation = generation;
        state.requests.queue.clear();
    }

    fn set_request_prefix(&mut self, prefix: u64) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .requests
            .prefix = prefix;
    }

    pub fn activate(&mut self) -> Result<(), ExtensionError> {
        self.call_unit("on_activate", ())?;
        self.rebuild_ui()
    }

    pub fn shutdown(&mut self) -> Result<(), ExtensionError> {
        self.call_unit("on_shutdown", ())
    }

    pub fn rebuild_ui(&mut self) -> Result<(), ExtensionError> {
        let function_ast = self.ui_ast.as_ref().unwrap_or(&self.ast);
        let value = self
            .engine
            .call_fn::<Dynamic>(&mut self.scope, function_ast, "build_ui", ())
            .map_err(|error| ExtensionError::Script(error.to_string()))?;
        self.ui = Some(parse_ui(&self.manifest, value, self.limits.max_ui_nodes)?);
        Ok(())
    }

    pub fn ui(&self) -> Option<&UiDescription> {
        self.ui.as_ref()
    }

    pub fn handle_event(&mut self, event: UiEvent) -> Result<(), ExtensionError> {
        self.call_unit(
            "on_ui_event",
            (event.widget_id, json_to_dynamic(event.value)),
        )?;
        self.rebuild_ui()
    }

    pub fn handle_response(&mut self, response: OperationResponse) -> Result<(), ExtensionError> {
        if response.generation != self.generation() {
            return Ok(());
        }
        let value = match response.result {
            Ok(value) => json_to_dynamic(value),
            Err(error) => {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                state.last_error = Some(error.message);
                Dynamic::UNIT
            }
        };
        self.call_unit("on_operation_result", (response.id as i64, value))?;
        self.rebuild_ui()
    }

    pub fn handle_event_message(&mut self, event: ExtensionEvent) -> Result<(), ExtensionError> {
        match event {
            ExtensionEvent::OperationResponse(response) => self.handle_response(response),
            ExtensionEvent::Ui(event) => self.handle_event(event),
            ExtensionEvent::Subscription { topic, payload } => {
                self.call_unit("on_subscription", (topic, json_to_dynamic(payload)))?;
                self.rebuild_ui()
            }
            ExtensionEvent::Activated => self.activate(),
            ExtensionEvent::Shutdown => self.shutdown(),
        }
    }

    pub fn take_requests(&self) -> Vec<OperationRequest> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.requests.queue.drain(..).collect()
    }

    pub fn invoke(&mut self, operation: &str, arguments: Value) -> Result<u64, ExtensionError> {
        if !self
            .manifest
            .operations
            .iter()
            .any(|candidate| candidate.id == operation)
        {
            return Err(ExtensionError::Package(format!(
                "unknown operation {operation:?}"
            )));
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        enqueue_request(&mut state, &self.manifest.id, operation, arguments)
    }

    pub fn last_error(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .last_error
            .clone()
    }

    fn call_unit<A: rhai::FuncArgs>(&mut self, name: &str, args: A) -> Result<(), ExtensionError> {
        self.engine
            .call_fn::<Dynamic>(&mut self.scope, &self.ast, name, args)
            .map(|_| ())
            .map_err(|error| ExtensionError::Script(error.to_string()))
    }
}

/// Multi-extension session manager. It owns a generation and rejects stale
/// operation responses before they can reach a script.
#[derive(Default)]
pub struct ExtensionHost {
    generation: u64,
    registry: OperationRegistry,
    instances: BTreeMap<String, ExtensionInstance>,
    /// Package order is significant for lifecycle hooks: extensions may
    /// inject Ruby modules that later extensions depend on. Keep the map for
    /// stable lookup/UI ordering, but use this list for activation and request
    /// delivery.
    load_order: Vec<String>,
    request_owners: HashMap<u64, String>,
    shutdown_requests: Vec<OperationRequest>,
    activated: bool,
    capabilities: Option<std::collections::BTreeSet<String>>,
    next_request_prefix: u64,
}

impl ExtensionHost {
    pub fn new() -> Self {
        Self {
            generation: 1,
            next_request_prefix: 1,
            ..Default::default()
        }
    }

    /// Configure the exact host capabilities. An unconfigured host remains
    /// permissive for compatibility with existing sessions.
    pub fn with_capabilities<I, S>(capabilities: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut host = Self::new();
        host.capabilities = Some(capabilities.into_iter().map(Into::into).collect());
        host
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn operation_registry(&self) -> &OperationRegistry {
        &self.registry
    }

    pub fn load(&mut self, package: ExtensionPackage) -> Result<(), ExtensionError> {
        let id = package.manifest.id.clone();
        if let Some(capabilities) = &self.capabilities {
            for required in package
                .manifest
                .operations
                .iter()
                .flat_map(|operation| &operation.required_capabilities)
                .chain(package.manifest.capabilities.iter())
            {
                if !capabilities.contains(required) {
                    return Err(ExtensionError::UnsupportedCapability(required.clone()));
                }
            }
        }
        if self.instances.contains_key(&id) {
            self.unload(&id)?;
        }
        let mut instance = ExtensionInstance::load(package)?;
        self.registry
            .register_manifest(&instance.manifest)
            .map_err(|error| ExtensionError::Api(error.to_string()))?;
        instance.set_generation(self.generation);
        instance.set_request_prefix(self.next_request_prefix);
        self.next_request_prefix = self.next_request_prefix.saturating_add(1_000_000);
        if self.activated {
            instance.activate()?;
        }
        self.load_order.push(id.clone());
        self.instances.insert(id, instance);
        Ok(())
    }

    /// Loads a single-file Rhai plugin supplied by an importer or paste
    /// action, using the same validation and lifecycle as package directories.
    pub fn load_rhai(
        &mut self,
        id: impl Into<String>,
        source: String,
    ) -> Result<(), ExtensionError> {
        self.load(ExtensionPackage::from_rhai(id, source)?)
    }

    pub fn unload(&mut self, id: &str) -> Result<(), ExtensionError> {
        self.load_order.retain(|loaded| loaded != id);
        let result = if let Some(mut instance) = self.instances.remove(id) {
            let result = instance.shutdown();
            let shutdown_requests = instance.take_requests();
            self.request_owners.retain(|_, owner| owner != id);
            for request in shutdown_requests {
                self.request_owners.insert(request.id, id.to_owned());
                self.shutdown_requests.push(request);
            }
            result
        } else {
            Ok(())
        };
        self.registry.remove_extension(id);
        result
    }

    pub fn activate_all(&mut self) -> Result<(), ExtensionError> {
        let load_order = self.load_order.clone();
        for id in load_order {
            if let Some(instance) = self.instances.get_mut(&id) {
                instance.activate()?;
            }
        }
        self.activated = true;
        Ok(())
    }

    pub fn reset_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1);
        self.request_owners.clear();
        for instance in self.instances.values_mut() {
            instance.set_generation(self.generation);
        }
    }

    pub fn get(&self, id: &str) -> Option<&ExtensionInstance> {
        self.instances.get(id)
    }

    /// Returns whether an extension ID is already loaded. Native hosts use
    /// this to keep an external package directory from replacing an embedded
    /// bundle accidentally.
    pub fn contains(&self, id: &str) -> bool {
        self.instances.contains_key(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut ExtensionInstance> {
        self.instances.get_mut(id)
    }

    pub fn ui(&self) -> impl Iterator<Item = &UiDescription> {
        self.instances.values().filter_map(ExtensionInstance::ui)
    }

    /// Returns HTTP aliases owned by loaded extensions. The core transport
    /// registers these patterns without knowing their game semantics; the
    /// injected Ruby module remains responsible for validation and results.
    pub fn route_aliases(&self) -> Vec<(String, String, String, Vec<String>)> {
        self.instances
            .values()
            .flat_map(|instance| {
                instance.manifest.operations.iter().flat_map(|operation| {
                    operation.route_aliases.iter().map(|alias| {
                        (
                            alias.method.clone(),
                            alias.path.clone(),
                            operation.id.clone(),
                            alias.arguments.clone(),
                        )
                    })
                })
            })
            .collect()
    }

    pub fn take_requests(&mut self) -> Vec<OperationRequest> {
        let mut requests = std::mem::take(&mut self.shutdown_requests);
        for owner in &self.load_order {
            if let Some(instance) = self.instances.get_mut(owner) {
                for request in instance.take_requests() {
                    self.request_owners.insert(request.id, owner.clone());
                    requests.push(request);
                }
            }
        }
        requests
    }

    pub fn dispatch_response(&mut self, response: OperationResponse) -> Result<(), ExtensionError> {
        if response.generation != self.generation {
            return Ok(());
        }
        let Some(owner) = self.request_owners.remove(&response.id) else {
            return Ok(());
        };
        if let Some(instance) = self.instances.get_mut(&owner) {
            instance.handle_response(response)
        } else {
            // Shutdown requests can still be in flight after their extension
            // has been removed. Their response only completes native cleanup;
            // there is no script left to resume.
            Ok(())
        }
    }

    pub fn dispatch_ui_event(&mut self, event: UiEvent) -> Result<(), ExtensionError> {
        let owner = event
            .panel_id
            .split_once('.')
            .map_or(event.panel_id.as_str(), |(owner, _)| owner);
        self.instances
            .get_mut(owner)
            .ok_or_else(|| ExtensionError::Package(format!("unknown plugin panel {owner:?}")))?
            .handle_event(event)
    }

    /// Invokes a registered operation through the same bounded queue used by
    /// Rhai, allowing HTTP and MCP adapters to share one execution path.
    pub fn invoke_operation(
        &mut self,
        extension_id: &str,
        operation: &str,
        arguments: Value,
    ) -> Result<u64, ExtensionError> {
        let id = self
            .instances
            .get_mut(extension_id)
            .ok_or_else(|| ExtensionError::Package(format!("unknown plugin {extension_id:?}")))?
            .invoke(operation, arguments)?;
        self.request_owners.insert(id, extension_id.into());
        Ok(id)
    }
}

fn register_host_functions(
    engine: &mut Engine,
    state: &Arc<Mutex<ScriptState>>,
    extension_id: &str,
    allow_ruby_inject: bool,
    allow_ruby_unload: bool,
    allow_webview: bool,
    ruby_source: Option<String>,
    text_sources: BTreeMap<String, String>,
) {
    let state_for_request = Arc::clone(state);
    let extension_id_for_request = extension_id.to_owned();
    engine.register_fn(
        "request",
        move |operation: ImmutableString, arguments: Map| -> i64 {
            let mut state = state_for_request
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            enqueue_request(
                &mut state,
                &extension_id_for_request,
                &operation,
                map_to_json(&arguments),
            )
            .map_or(-1, |id| id as i64)
        },
    );

    let state_for_get = Arc::clone(state);
    engine.register_fn("state_get", move |key: ImmutableString| -> Dynamic {
        state_for_get
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values
            .get(key.as_str())
            .cloned()
            .unwrap_or(Dynamic::UNIT)
    });

    let state_for_set = Arc::clone(state);
    engine.register_fn("state_set", move |key: ImmutableString, value: Dynamic| {
        state_for_set
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values
            .insert(key.into(), value);
    });

    let state_for_error = Arc::clone(state);
    engine.register_fn("last_error", move || -> String {
        state_for_error
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .last_error
            .clone()
            .unwrap_or_default()
    });

    let state_for_ruby_inject = Arc::clone(state);
    let extension_id_for_ruby_inject = extension_id.to_owned();
    engine.register_fn(
        "ruby_inject",
        move |name: ImmutableString, source: ImmutableString| -> i64 {
            let mut state = state_for_ruby_inject
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !allow_ruby_inject {
                state.last_error = Some("plugin lacks ruby.inject capability".into());
                return -1;
            }
            if name.trim().is_empty() || source.trim().is_empty() {
                state.last_error = Some("ruby_inject requires a name and source".into());
                return -1;
            }
            enqueue_request(
                &mut state,
                &extension_id_for_ruby_inject,
                "__ruby.inject",
                serde_json::json!({
                    "name": name.to_string(),
                    "source": source.to_string(),
                }),
            )
            .map_or(-1, |id| id as i64)
        },
    );

    let state_for_ruby_code = Arc::clone(state);
    let extension_id_for_ruby_code = extension_id.to_owned();
    engine.register_fn(
        "ruby_inject_source",
        move |name: ImmutableString, apply: ImmutableString, revert: ImmutableString| -> i64 {
            let mut state = state_for_ruby_code
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !allow_ruby_inject {
                state.last_error = Some("plugin lacks ruby.inject capability".into());
                return -1;
            }
            if name.trim().is_empty() || apply.trim().is_empty() {
                state.last_error =
                    Some("ruby_inject_source requires a name and apply source".into());
                return -1;
            }
            let patch = format!("{{ apply: proc {{ {apply} }}, revert: proc {{ {revert} }} }}");
            enqueue_request(
                &mut state,
                &extension_id_for_ruby_code,
                "__ruby.inject",
                serde_json::json!({
                    "name": name.to_string(),
                    "source": patch,
                }),
            )
            .map_or(-1, |id| id as i64)
        },
    );

    let state_for_ruby_unload = Arc::clone(state);
    let extension_id_for_ruby_unload = extension_id.to_owned();
    engine.register_fn("ruby_unload", move |handle: i64| -> i64 {
        let mut state = state_for_ruby_unload
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !allow_ruby_unload {
            state.last_error = Some("plugin lacks ruby.unload capability".into());
            return -1;
        }
        if handle <= 0 {
            state.last_error = Some("ruby_unload requires a positive handle".into());
            return -1;
        }
        enqueue_request(
            &mut state,
            &extension_id_for_ruby_unload,
            "__ruby.unload",
            serde_json::json!({ "handle": handle }),
        )
        .map_or(-1, |id| id as i64)
    });

    let state_for_webview_open = Arc::clone(state);
    let extension_id_for_webview_open = extension_id.to_owned();
    engine.register_fn(
        "webview_open",
        move |view_id: ImmutableString, title: ImmutableString, html: ImmutableString| -> i64 {
            let mut state = state_for_webview_open
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !allow_webview {
                state.last_error = Some("plugin lacks ui.webview capability".into());
                return -1;
            }
            if view_id.trim().is_empty() || title.trim().is_empty() || html.trim().is_empty() {
                state.last_error = Some("webview_open requires a view id, title, and HTML".into());
                return -1;
            }
            enqueue_request(
                &mut state,
                &extension_id_for_webview_open,
                "__webview.open",
                serde_json::json!({
                    "view_id": view_id.to_string(),
                    "title": title.to_string(),
                    "html": html.to_string(),
                }),
            )
            .map_or(-1, |id| id as i64)
        },
    );

    let state_for_webview_close = Arc::clone(state);
    let extension_id_for_webview_close = extension_id.to_owned();
    engine.register_fn("webview_close", move |view_id: ImmutableString| -> i64 {
        let mut state = state_for_webview_close
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !allow_webview {
            state.last_error = Some("plugin lacks ui.webview capability".into());
            return -1;
        }
        if view_id.trim().is_empty() {
            state.last_error = Some("webview_close requires a view id".into());
            return -1;
        }
        enqueue_request(
            &mut state,
            &extension_id_for_webview_close,
            "__webview.close",
            serde_json::json!({ "view_id": view_id.to_string() }),
        )
        .map_or(-1, |id| id as i64)
    });

    let ruby_source = ruby_source.unwrap_or_default();
    engine.register_fn("ruby_source", move || -> String { ruby_source.clone() });

    let text_sources_for_block = text_sources;
    engine.register_fn("text_block", move |name: ImmutableString| -> String {
        text_sources_for_block
            .get(name.as_str())
            .cloned()
            .unwrap_or_default()
    });

    engine.register_fn(
        "ui_label",
        |id: ImmutableString, text: ImmutableString| -> Map {
            node_map("label", id, [("text", Dynamic::from(text))])
        },
    );
    engine.register_fn(
        "ui_button",
        |id: ImmutableString, label: ImmutableString| -> Map {
            node_map("button", id, [("label", Dynamic::from(label))])
        },
    );
    engine.register_fn(
        "ui_checkbox",
        |id: ImmutableString, label: ImmutableString, value: bool| -> Map {
            node_map(
                "checkbox",
                id,
                [
                    ("label", Dynamic::from(label)),
                    ("value", Dynamic::from(value)),
                ],
            )
        },
    );
    engine.register_fn(
        "ui_text_input",
        |id: ImmutableString, label: ImmutableString, value: ImmutableString| -> Map {
            node_map(
                "text_input",
                id,
                [
                    ("label", Dynamic::from(label)),
                    ("value", Dynamic::from(value)),
                ],
            )
        },
    );
    engine.register_fn(
        "ui_slider",
        |id: ImmutableString,
         label: ImmutableString,
         value: f64,
         min: f64,
         max: f64,
         step: f64,
         suffix: ImmutableString|
         -> Map {
            node_map(
                "slider",
                id,
                [
                    ("label", Dynamic::from(label)),
                    ("value", Dynamic::from(value)),
                    ("min", Dynamic::from(min)),
                    ("max", Dynamic::from(max)),
                    ("step", Dynamic::from(step)),
                    ("suffix", Dynamic::from(suffix)),
                ],
            )
        },
    );
    engine.register_fn(
        "ui_loading",
        |id: ImmutableString, label: ImmutableString| -> Map {
            node_map("loading", id, [("label", Dynamic::from(label))])
        },
    );
    engine.register_fn(
        "ui_group",
        |id: ImmutableString, title: ImmutableString, children: Array| -> Map {
            node_map(
                "group",
                id,
                [
                    ("title", Dynamic::from(title)),
                    ("children", Dynamic::from(children)),
                ],
            )
        },
    );
    engine.register_fn("ui_row", |id: ImmutableString, children: Array| -> Map {
        node_map("row", id, [("children", Dynamic::from(children))])
    });
}

fn enqueue_request(
    state: &mut ScriptState,
    extension_id: &str,
    operation: &str,
    arguments: Value,
) -> Result<u64, ExtensionError> {
    if state.requests.queue.len() >= state.requests.max_requests {
        state.last_error = Some("plugin request queue is full".into());
        return Err(ExtensionError::Limit("request queue is full".into()));
    }
    state.requests.next_id = state.requests.next_id.wrapping_add(1).max(1);
    let id = state.requests.prefix.saturating_add(state.requests.next_id);
    let generation = state.requests.generation;
    state.requests.queue.push_back(OperationRequest {
        id,
        generation,
        operation: format!("{extension_id}.{operation}"),
        arguments,
    });
    Ok(id)
}

fn node_map(
    kind: &str,
    id: ImmutableString,
    fields: impl IntoIterator<Item = (&'static str, Dynamic)>,
) -> Map {
    let mut map = Map::new();
    map.insert("type".into(), kind.into());
    map.insert("id".into(), id.into());
    for (key, value) in fields {
        map.insert(key.into(), value);
    }
    map
}

fn map_to_json(map: &Map) -> Value {
    let mut output = JsonMap::new();
    for (key, value) in map {
        output.insert(key.to_string(), dynamic_to_json(value));
    }
    Value::Object(output)
}

fn dynamic_to_json(value: &Dynamic) -> Value {
    if value.is_unit() {
        Value::Null
    } else if let Some(value) = value.clone().try_cast::<bool>() {
        Value::Bool(value)
    } else if let Some(value) = value.clone().try_cast::<i64>() {
        Value::Number(value.into())
    } else if let Some(value) = value.clone().try_cast::<f64>() {
        serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number)
    } else if let Some(value) = value.clone().try_cast::<ImmutableString>() {
        Value::String(value.to_string())
    } else if let Some(value) = value.clone().try_cast::<Array>() {
        Value::Array(value.iter().map(dynamic_to_json).collect())
    } else if let Some(value) = value.clone().try_cast::<Map>() {
        map_to_json(&value)
    } else {
        Value::Null
    }
}

fn json_to_dynamic(value: Value) -> Dynamic {
    match value {
        Value::Null => Dynamic::UNIT,
        Value::Bool(value) => value.into(),
        Value::Number(value) => value
            .as_i64()
            .map_or_else(|| value.as_f64().unwrap_or_default().into(), Dynamic::from),
        Value::String(value) => value.into(),
        Value::Array(values) => values
            .into_iter()
            .map(json_to_dynamic)
            .collect::<Array>()
            .into(),
        Value::Object(values) => values
            .into_iter()
            .map(|(key, value)| (key.into(), json_to_dynamic(value)))
            .collect::<Map>()
            .into(),
    }
}

fn parse_ui(
    manifest: &ExtensionManifest,
    value: Dynamic,
    max_nodes: usize,
) -> Result<UiDescription, ExtensionError> {
    let panel = manifest.panels.first();
    let mut count = 0;
    let nodes = value
        .try_cast::<Array>()
        .ok_or_else(|| ExtensionError::Script("build_ui must return an array".into()))?
        .into_iter()
        .map(|node| parse_node(node, &mut count, max_nodes))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(UiDescription {
        panel_id: format!(
            "{}.{}",
            manifest.id,
            panel.map_or("main", |panel| panel.id.as_str())
        ),
        title: panel.map_or_else(|| manifest.name.clone(), |panel| panel.title.clone()),
        nodes,
        error: None,
    })
}

fn parse_node(
    value: Dynamic,
    count: &mut usize,
    max_nodes: usize,
) -> Result<UiNode, ExtensionError> {
    *count += 1;
    if *count > max_nodes {
        return Err(ExtensionError::Limit("UI node limit exceeded".into()));
    }
    let map = value
        .try_cast::<Map>()
        .ok_or_else(|| ExtensionError::Script("UI nodes must be maps".into()))?;
    let string = |key: &str| -> Result<String, ExtensionError> {
        map.get(key)
            .and_then(|value| value.clone().try_cast::<ImmutableString>())
            .map(|value| value.to_string())
            .ok_or_else(|| {
                ExtensionError::Script(format!("UI node is missing string field {key:?}"))
            })
    };
    let kind = string("type")?;
    let id = string("id")?;
    let node_id = id.clone();
    let mut children = |key: &str| -> Result<Vec<UiNode>, ExtensionError> {
        map.get(key)
            .and_then(|value| value.clone().try_cast::<Array>())
            .ok_or_else(|| {
                ExtensionError::Script(format!("UI node {node_id:?} is missing children"))
            })?
            .into_iter()
            .map(|child| parse_node(child, count, max_nodes))
            .collect()
    };
    match kind.as_str() {
        "label" => Ok(UiNode::Label {
            id,
            text: string("text")?,
        }),
        "button" => Ok(UiNode::Button {
            id,
            label: string("label")?,
        }),
        "checkbox" => Ok(UiNode::Checkbox {
            id,
            label: string("label")?,
            value: map
                .get("value")
                .and_then(|value| value.clone().try_cast::<bool>())
                .unwrap_or(false),
        }),
        "text_input" => Ok(UiNode::TextInput {
            id,
            label: string("label")?,
            value: string("value")?,
        }),
        "slider" => Ok(UiNode::Slider {
            id,
            label: string("label")?,
            value: number(&map, "value")?,
            min: number(&map, "min")?,
            max: number(&map, "max")?,
            step: number(&map, "step")?,
            suffix: string("suffix")?,
        }),
        "loading" => Ok(UiNode::Loading {
            id,
            label: string("label")?,
        }),
        "group" => Ok(UiNode::Group {
            id,
            title: string("title")?,
            children: children("children")?,
        }),
        "row" => Ok(UiNode::Row {
            id,
            children: children("children")?,
        }),
        _ => Err(ExtensionError::Script(format!(
            "unknown UI node type {kind:?}"
        ))),
    }
}

fn number(map: &Map, key: &str) -> Result<f64, ExtensionError> {
    map.get(key)
        .and_then(|value| {
            value
                .clone()
                .try_cast::<f64>()
                .or_else(|| value.clone().try_cast::<i64>().map(|value| value as f64))
        })
        .ok_or_else(|| ExtensionError::Script(format!("UI node is missing number field {key:?}")))
}

#[derive(Debug)]
pub enum ExtensionError {
    Package(String),
    Manifest(ManifestError),
    Script(String),
    Limit(String),
    UnsupportedCapability(String),
    Api(String),
}

impl fmt::Display for ExtensionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Package(message) => write!(f, "plugin package error: {message}"),
            Self::Manifest(error) => write!(f, "plugin manifest error: {error}"),
            Self::Script(message) => write!(f, "plugin script error: {message}"),
            Self::Limit(message) => write!(f, "plugin limit exceeded: {message}"),
            Self::UnsupportedCapability(capability) => {
                write!(f, "plugin capability is unavailable: {capability}")
            }
            Self::Api(message) => write!(f, "plugin API registry error: {message}"),
        }
    }
}

impl std::error::Error for ExtensionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use fanplay_extension_api::{ExtensionLimits, OperationError, OperationResponse, UiNode};

    fn package() -> ExtensionPackage {
        ExtensionPackage::from_rhai_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../extensions/essentials-debug/main.rhai"
        ))
        .unwrap()
    }

    fn ruby_package() -> ExtensionPackage {
        ExtensionPackage::from_rhai(
            "ruby-test",
            r#"
// fanplay:capabilities = ruby.inject,ruby.unload
fn on_activate() {
    state_set("handle", ruby_inject_source("demo", "module Demo; end", "Object.send(:remove_const, :Demo) if defined?(Demo)"));
}
fn build_ui() { [] }
fn on_ui_event(_id, _value) { }
fn on_operation_result(id, _result) {
    if id == state_get("handle") {
        state_set("unload", ruby_unload(id));
    }
}
fn on_shutdown() { }
"#
            .into(),
        )
        .unwrap()
    }

    fn inventory_package() -> ExtensionPackage {
        ExtensionPackage::from_rhai_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../extensions/essentials-inventory/main.rhai"
        ))
        .unwrap()
    }

    fn game_api_package() -> ExtensionPackage {
        ExtensionPackage::from_rhai_with_ruby(
            "essentials-game-api",
            std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../extensions/essentials-game-api/main.rhai"
            ))
            .unwrap(),
            Some(
                std::fs::read_to_string(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../extensions/essentials-game-api/game_api.rb"
                ))
                .unwrap(),
            ),
        )
        .unwrap()
    }

    #[test]
    fn bundled_runtime_and_pokeapi_plugins_parse() {
        let mut host = ExtensionHost::new();
        for path in [
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../extensions/essentials-runtime-patches.rhai"
            ),
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../extensions/pokeapi.rhai"),
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../extensions/essentials-online-play.rhai"
            ),
        ] {
            let package = ExtensionPackage::from_rhai_file(path).unwrap();
            assert!(!package.manifest.id.is_empty());
            host.load(package).unwrap();
        }
        host.activate_all().unwrap();
        assert_eq!(host.ui().count(), 3);
    }

    fn dynamic_demo_package() -> ExtensionPackage {
        ExtensionPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../extensions/dynamic-demo.rhai"
        ))
        .unwrap()
    }

    fn pokedex_package() -> ExtensionPackage {
        ExtensionPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../extensions/essentials-pokedex.rhai"
        ))
        .unwrap()
    }

    fn movedex_package() -> ExtensionPackage {
        ExtensionPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../extensions/essentials-movedex.rhai"
        ))
        .unwrap()
    }

    fn itemdex_package() -> ExtensionPackage {
        ExtensionPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../extensions/essentials-itemdex.rhai"
        ))
        .unwrap()
    }

    fn encounters_package() -> ExtensionPackage {
        ExtensionPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../extensions/essentials-encounters.rhai"
        ))
        .unwrap()
    }

    fn world_map_package() -> ExtensionPackage {
        ExtensionPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../extensions/essentials-world-map.rhai"
        ))
        .unwrap()
    }

    #[test]
    fn activates_builds_ui_and_queues_namespaced_requests() {
        let mut host = ExtensionHost::new();
        host.load(package()).unwrap();
        host.activate_all().unwrap();
        let requests = host.take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].operation,
            "pokemon-essentials-debug.pokemon_essentials_debug"
        );
        assert_eq!(requests[0].generation, host.generation());
        let ui = host.get("pokemon-essentials-debug").unwrap().ui().unwrap();
        assert!(matches!(ui.nodes[0], UiNode::Loading { .. }));
    }

    #[test]
    fn responses_resume_the_script_and_rebuild_the_ui() {
        let mut host = ExtensionHost::new();
        host.load(package()).unwrap();
        host.activate_all().unwrap();
        let request = host.take_requests().pop().unwrap();
        host.dispatch_response(OperationResponse {
            id: request.id,
            generation: request.generation,
            result: Ok(serde_json::json!({"debug": true})),
        })
        .unwrap();
        let ui = host.get("pokemon-essentials-debug").unwrap().ui().unwrap();
        assert!(matches!(ui.nodes[0], UiNode::Checkbox { value: true, .. }));
    }

    #[test]
    fn debug_toggle_uses_the_ruby_debug_field() {
        let mut host = ExtensionHost::new();
        host.load(package()).unwrap();
        host.activate_all().unwrap();
        let query = host.take_requests().pop().unwrap();
        host.dispatch_response(OperationResponse {
            id: query.id,
            generation: query.generation,
            result: Ok(serde_json::json!({"debug": false})),
        })
        .unwrap();
        host.dispatch_ui_event(UiEvent {
            panel_id: "pokemon-essentials-debug.debug".into(),
            widget_id: "debug.enabled".into(),
            value: serde_json::json!(true),
        })
        .unwrap();
        let request = host.take_requests().pop().unwrap();
        assert_eq!(
            request.operation,
            "pokemon-essentials-debug.pokemon_essentials_debug"
        );
        assert_eq!(request.arguments["enabled"], serde_json::json!(true));
    }

    #[test]
    fn stale_responses_are_ignored_after_generation_reset() {
        let mut host = ExtensionHost::new();
        host.load(package()).unwrap();
        host.activate_all().unwrap();
        let request = host.take_requests().pop().unwrap();
        host.reset_generation();
        host.dispatch_response(OperationResponse {
            id: request.id,
            generation: request.generation,
            result: Err(OperationError::new("stale", "old session")),
        })
        .unwrap();
        assert!(host
            .get("pokemon-essentials-debug")
            .unwrap()
            .last_error()
            .is_none());
    }

    #[test]
    fn configured_hosts_reject_missing_capabilities() {
        let mut host = ExtensionHost::with_capabilities(["unrelated"]);
        assert!(matches!(
            host.load(package()),
            Err(ExtensionError::UnsupportedCapability(_))
        ));
    }

    #[test]
    fn request_queue_saturation_is_reported_without_running_more_script() {
        let instance = ExtensionInstance::load_with_limits(
            package(),
            ExtensionLimits {
                max_requests: 0,
                ..Default::default()
            },
        )
        .unwrap();
        let mut instance = instance;
        instance.activate().unwrap();
        assert!(instance.take_requests().is_empty());
        assert_eq!(
            instance.last_error().as_deref(),
            Some("plugin request queue is full")
        );
    }

    #[test]
    fn unload_removes_the_instance_and_its_pending_owner() {
        let mut host = ExtensionHost::new();
        host.load(package()).unwrap();
        host.activate_all().unwrap();
        let request = host.take_requests().pop().unwrap();
        host.unload("pokemon-essentials-debug").unwrap();
        assert!(host.get("pokemon-essentials-debug").is_none());
        host.dispatch_response(OperationResponse {
            id: request.id,
            generation: request.generation,
            result: Ok(Value::Null),
        })
        .unwrap();
    }

    #[test]
    fn external_invocation_uses_the_same_script_request_queue() {
        let mut host = ExtensionHost::new();
        host.load(package()).unwrap();
        let id = host
            .invoke_operation(
                "pokemon-essentials-debug",
                "pokemon_essentials_debug",
                serde_json::json!({}),
            )
            .unwrap();
        let request = host.take_requests().pop().unwrap();
        assert_eq!(request.id, id);
        host.dispatch_response(OperationResponse {
            id,
            generation: request.generation,
            result: Ok(serde_json::json!({"enabled": false})),
        })
        .unwrap();
        assert!(matches!(
            host.get("pokemon-essentials-debug")
                .unwrap()
                .ui()
                .unwrap()
                .nodes[0],
            UiNode::Checkbox { value: false, .. }
        ));
    }

    #[test]
    fn ruby_primitives_queue_reversible_mutations_and_unload_by_handle() {
        let mut host = ExtensionHost::new();
        host.load(ruby_package()).unwrap();
        host.activate_all().unwrap();
        let inject = host.take_requests().pop().unwrap();
        assert_eq!(inject.operation, "ruby-test.__ruby.inject");
        assert_eq!(inject.arguments["name"], "demo");
        assert!(inject.arguments["source"]
            .as_str()
            .unwrap()
            .contains("module Demo; end"));

        host.dispatch_response(OperationResponse {
            id: inject.id,
            generation: inject.generation,
            result: Ok(serde_json::json!({"patch_id": "user/rhai-1"})),
        })
        .unwrap();
        let unload = host.take_requests().pop().unwrap();
        assert_eq!(unload.operation, "ruby-test.__ruby.unload");
        assert_eq!(unload.arguments["handle"], inject.id as i64);
    }

    #[test]
    fn ruby_primitives_are_capability_gated() {
        let package = ruby_package();
        let mut host = ExtensionHost::with_capabilities(["ruby.inject"]);
        assert!(matches!(
            host.load(package),
            Err(ExtensionError::UnsupportedCapability(capability)) if capability == "ruby.unload"
        ));
    }

    #[test]
    fn inventory_extension_keeps_form_state_and_starts_without_requests() {
        let mut host = ExtensionHost::new();
        host.load(inventory_package()).unwrap();
        host.activate_all().unwrap();
        assert!(host.take_requests().is_empty());
        let ui = host.get("essentials-inventory").unwrap().ui().unwrap();
        assert_eq!(ui.panel_id, "essentials-inventory.inventory");
        assert!(ui
            .nodes
            .iter()
            .any(|node| matches!(node, UiNode::Group { .. })));
    }

    #[test]
    fn game_api_extension_injects_ruby_module_then_queries_status() {
        let mut host = ExtensionHost::new();
        host.load(game_api_package()).unwrap();
        host.activate_all().unwrap();
        let inject = host.take_requests().pop().unwrap();
        assert_eq!(inject.operation, "essentials-game-api.__ruby.inject");
        assert!(inject.arguments["source"]
            .as_str()
            .unwrap()
            .contains("module FANPLAY"));

        host.dispatch_response(OperationResponse {
            id: inject.id,
            generation: inject.generation,
            result: Ok(serde_json::json!({"patch_id": 123, "registered": true})),
        })
        .unwrap();
        let status = host.take_requests().pop().unwrap();
        assert_eq!(
            status.operation,
            "essentials-game-api.pokemon_essentials_status"
        );
        host.dispatch_response(OperationResponse {
            id: status.id,
            generation: status.generation,
            result: Ok(serde_json::json!({"available": false})),
        })
        .unwrap();
        let retry = host.take_requests().pop().unwrap();
        assert_eq!(
            retry.operation,
            "essentials-game-api.pokemon_essentials_status"
        );
        host.dispatch_response(OperationResponse {
            id: retry.id,
            generation: retry.generation,
            result: Ok(serde_json::json!({"available": true})),
        })
        .unwrap();
        let ui = host.get("essentials-game-api").unwrap().ui().unwrap();
        assert!(ui.nodes.iter().any(|node| matches!(
            node,
            UiNode::Label { text, .. } if text == "Essentials is active."
        )));
    }

    #[test]
    fn ruby_extension_requests_follow_load_order() {
        let mut host = ExtensionHost::new();
        // `essentials-encounters` sorts before `essentials-game-api`; the
        // latter must still inject first because encounters extends it.
        host.load(game_api_package()).unwrap();
        host.load(encounters_package()).unwrap();
        host.activate_all().unwrap();

        let requests = host.take_requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].operation,
            "essentials-game-api.__ruby.inject"
        );
        assert_eq!(
            requests[1].operation,
            "essentials-encounters.__ruby.inject"
        );
    }

    #[test]
    fn filesystem_dynamic_demo_package_loads_and_updates_ui() {
        let mut host = ExtensionHost::new();
        host.load(dynamic_demo_package()).unwrap();
        host.activate_all().unwrap();
        let ui = host.get("dynamic-demo").unwrap().ui().unwrap();
        assert!(ui.nodes.iter().any(
            |node| matches!(node, UiNode::Button { id, .. } if id == "dynamic-demo.increment")
        ));
        host.dispatch_ui_event(UiEvent {
            panel_id: "dynamic-demo.demo".into(),
            widget_id: "dynamic-demo.increment".into(),
            value: Value::Null,
        })
        .unwrap();
        let ui = host.get("dynamic-demo").unwrap().ui().unwrap();
        assert!(ui.nodes.iter().any(
            |node| matches!(node, UiNode::Label { id, text } if id == "dynamic-demo.count" && text.contains("1"))
        ));
    }

    #[test]
    fn webview_plugin_queues_an_html_view_command() {
        let mut host = ExtensionHost::new();
        host.load(dynamic_demo_package()).unwrap();
        host.activate_all().unwrap();
        host.dispatch_ui_event(UiEvent {
            panel_id: "dynamic-demo.demo".into(),
            widget_id: "dynamic-demo.webview".into(),
            value: Value::Null,
        })
        .unwrap();
        let request = host.take_requests().pop().unwrap();
        assert_eq!(request.operation, "dynamic-demo.__webview.open");
        assert_eq!(request.arguments["view_id"], "dynamic-demo");
        assert!(request.arguments["html"]
            .as_str()
            .is_some_and(|html| html.contains("/v1/game/status")));
    }

    #[test]
    fn pokedex_is_a_single_file_plugin_with_embedded_ruby_and_html() {
        let package = pokedex_package();
        assert_eq!(package.manifest.id, "essentials-pokedex");
        assert!(package.text_sources.get("ruby").is_some_and(|ruby| {
            ruby.contains("module PokedexPlugin")
                && ruby.contains("sprite_data")
                && ruby.contains("shiny_sprite")
                && ruby.contains("pbResolveBitmap")
                && ruby.contains("pbCheckPokemonBitmapFiles")
                && ruby.contains("pbCheckPokemonIconFiles")
                && ruby.contains("asset_descriptor")
        }));
        assert!(package.text_sources.get("html").is_some_and(|html| {
            html.contains("Essentials Pokédex")
                && html.contains("<img id='sprite'")
                && html.contains("shiny-toggle")
                && html.contains("row-sprite")
                && html.contains("loadListArtwork")
                && html.contains("loadAssetImage")
                && html.contains("FANPLAY.asset")
                && html.contains("const ART_SIZE = 192")
        }));
        let mut host = ExtensionHost::new();
        host.load(package).unwrap();
        host.activate_all().unwrap();
        let inject = host.take_requests().pop().unwrap();
        assert_eq!(inject.operation, "essentials-pokedex.__ruby.inject");
        host.dispatch_response(OperationResponse {
            id: inject.id,
            generation: inject.generation,
            result: Ok(serde_json::json!({"patch_id": 42})),
        })
        .unwrap();
        host.dispatch_ui_event(UiEvent {
            panel_id: "essentials-pokedex.pokedex".into(),
            widget_id: "pokedex.open".into(),
            value: Value::Bool(true),
        })
        .unwrap();
        let open = host.take_requests().pop().unwrap();
        assert_eq!(open.operation, "essentials-pokedex.__webview.open");
        assert!(open.arguments["html"]
            .as_str()
            .is_some_and(|html| html.contains("Essentials Pokédex")
                && html.contains("/v1/pokemon-essentials/species/")));
    }

    #[test]
    fn movedex_is_a_single_file_html_plugin_with_plugin_owned_catalog() {
        let package = movedex_package();
        assert_eq!(package.manifest.id, "essentials-movedex");
        assert!(package
            .manifest
            .operations
            .iter()
            .any(|operation| operation.id == "pokemon_essentials_movedex"));
        assert!(package.text_sources.get("ruby").is_some_and(|ruby| {
            ruby.contains("module MovedexPlugin")
                && ruby.contains("def self.pokemon_essentials_movedex")
                && ruby.contains("legacy_catalog")
        }));
        assert!(package.main_source.contains("FANPLAY::MovedexPlugin.install"));
        assert!(package.text_sources.get("html").is_some_and(|html| {
            html.contains("Essentials Movedex")
                && html.contains("id=\"search\"")
                && html.contains("data-add")
                && html.contains("/v1/pokemon-essentials/movedex")
        }));

        let mut host = ExtensionHost::new();
        host.load(package).unwrap();
        host.activate_all().unwrap();
        let inject = host.take_requests().pop().unwrap();
        assert_eq!(inject.operation, "essentials-movedex.__ruby.inject");
        host.dispatch_response(OperationResponse {
            id: inject.id,
            generation: inject.generation,
            result: Ok(serde_json::json!({"patch_id": 7})),
        })
        .unwrap();
        host.dispatch_ui_event(UiEvent {
            panel_id: "essentials-movedex.movedex".into(),
            widget_id: "movedex.open".into(),
            value: Value::Bool(true),
        })
        .unwrap();
        let open = host.take_requests().pop().unwrap();
        assert_eq!(open.operation, "essentials-movedex.__webview.open");
        assert!(open.arguments["html"]
            .as_str()
            .is_some_and(|html| html.contains("/v1/pokemon-essentials/movedex")));
    }

    #[test]
    fn itemdex_is_a_single_file_html_plugin_with_plugin_owned_catalog() {
        let package = itemdex_package();
        assert_eq!(package.manifest.id, "essentials-itemdex");
        assert!(package
            .manifest
            .operations
            .iter()
            .any(|operation| operation.id == "pokemon_essentials_itemdex"));
        assert!(package.text_sources.get("ruby").is_some_and(|ruby| {
            ruby.contains("module ItemdexPlugin")
                && ruby.contains("def self.pokemon_essentials_itemdex")
                && ruby.contains("legacy_catalog")
                && ruby.contains("asset_descriptor")
        }));
        assert!(package.text_sources.get("html").is_some_and(|html| {
            html.contains("Essentials Itemdex")
                && html.contains("id=\"search\"")
                && html.contains("data-icon")
                && html.contains("/v1/pokemon-essentials/itemdex")
                && html.contains("/v1/pokemon-essentials/inventory/add")
        }));

        let mut host = ExtensionHost::new();
        host.load(package).unwrap();
        host.activate_all().unwrap();
        let inject = host.take_requests().pop().unwrap();
        assert_eq!(inject.operation, "essentials-itemdex.__ruby.inject");
        host.dispatch_response(OperationResponse {
            id: inject.id,
            generation: inject.generation,
            result: Ok(serde_json::json!({"patch_id": 8})),
        })
        .unwrap();
        host.dispatch_ui_event(UiEvent {
            panel_id: "essentials-itemdex.itemdex".into(),
            widget_id: "itemdex.open".into(),
            value: Value::Bool(true),
        })
        .unwrap();
        let open = host.take_requests().pop().unwrap();
        assert_eq!(open.operation, "essentials-itemdex.__webview.open");
        assert!(open.arguments["html"]
            .as_str()
            .is_some_and(|html| html.contains("Essentials Itemdex")
                && html.contains("/v1/pokemon-essentials/itemdex")));
    }

    #[test]
    fn encounters_plugin_opens_a_map_aware_html_view() {
        let package = encounters_package();
        assert_eq!(package.manifest.id, "essentials-encounters");
        assert!(package.text_sources.get("html").is_some_and(|html| {
            html.contains("Current Map Encounters")
                && html.contains("/v1/pokemon-essentials/status")
                && html.contains("/v1/pokemon-essentials/encounters/current-map")
                && html.contains("/v1/pokemon-essentials/pokedex")
                && html.contains("setInterval")
                && html.contains("mapKey")
                && html.contains("data-icon")
        }));
        assert!(package.text_sources.get("ruby").is_some_and(|ruby| {
            ruby.contains("module EncountersPlugin")
                && ruby.contains("def self.add_icons")
                && ruby.contains("PokedexPlugin.sprite_data")
                && ruby.contains("icon_descriptor")
        }));

        let mut host = ExtensionHost::new();
        host.load(package).unwrap();
        host.activate_all().unwrap();
        let inject = host.take_requests().pop().unwrap();
        assert_eq!(inject.operation, "essentials-encounters.__ruby.inject");
        host.dispatch_response(OperationResponse {
            id: inject.id,
            generation: inject.generation,
            result: Ok(serde_json::json!({"patch_id": 9})),
        })
        .unwrap();
        host.dispatch_ui_event(UiEvent {
            panel_id: "essentials-encounters.encounters".into(),
            widget_id: "encounters.open".into(),
            value: Value::Bool(true),
        })
        .unwrap();
        let open = host.take_requests().pop().unwrap();
        assert_eq!(open.operation, "essentials-encounters.__webview.open");
        assert!(open.arguments["html"].as_str().is_some_and(|html| {
            html.contains("/v1/pokemon-essentials/status")
                && html.contains("/v1/pokemon-essentials/encounters/current-map")
        }));
    }

    #[test]
    fn world_map_is_a_single_file_html_plugin_with_plugin_owned_routes() {
        let package = world_map_package();
        assert_eq!(package.manifest.id, "essentials-world-map");
        assert!(package
            .manifest
            .operations
            .iter()
            .any(|operation| operation.id == "world_map"));
        assert!(package
            .manifest
            .operations
            .iter()
            .any(|operation| operation.id == "map_preview"));
        assert!(package.main_source.contains("FANPLAY::WorldMapPlugin.install"));
        assert!(package.text_sources.get("ruby").is_some_and(|ruby| {
            ruby.contains("module WorldMapPlugin")
                && ruby.contains("def self.world_map")
                && ruby.contains("def self.map_preview")
        }));
        assert!(package.text_sources.get("html").is_some_and(|html| {
            html.contains("Connected World Map")
                && html.contains("id=\"world\"")
                && html.contains("/v1/game/world-map")
                && html.contains("/v1/game/maps/")
        }));

        let mut host = ExtensionHost::new();
        host.load(package).unwrap();
        host.activate_all().unwrap();
        let inject = host.take_requests().pop().unwrap();
        assert_eq!(inject.operation, "essentials-world-map.__ruby.inject");
        host.dispatch_response(OperationResponse {
            id: inject.id,
            generation: inject.generation,
            result: Ok(serde_json::json!({"patch_id": 8})),
        })
        .unwrap();
        host.dispatch_ui_event(UiEvent {
            panel_id: "essentials-world-map.world-map".into(),
            widget_id: "world-map.open".into(),
            value: Value::Bool(true),
        })
        .unwrap();
        let open = host.take_requests().pop().unwrap();
        assert_eq!(open.operation, "essentials-world-map.__webview.open");
    }

    #[test]
    fn pasted_rhai_can_declare_identity_and_capabilities_without_a_manifest() {
        let package = ExtensionPackage::from_rhai(
            "pasted-demo",
            "// fanplay:name = Pasted Demo\n// fanplay:capabilities = ruby.inject,ruby.unload\n// fanplay:operation = ping\n// fanplay:route = GET /v1/ping ping value\nfn build_ui() { [] }\nfn on_activate() {}\nfn on_ui_event(_id, _value) {}\nfn on_operation_result(_id, _result) {}\nfn on_shutdown() {}".into(),
        )
        .unwrap();
        assert_eq!(package.manifest.name, "Pasted Demo");
        assert_eq!(
            package.manifest.capabilities,
            ["ruby.inject", "ruby.unload"]
        );
        assert_eq!(package.manifest.operations[0].id, "ping");
        assert_eq!(
            package.manifest.operations[0].route_aliases[0].path,
            "/v1/ping"
        );
    }

    #[test]
    fn a_single_rhai_file_can_embed_named_text() {
        let package = ExtensionPackage::from_rhai(
            "embedded-ruby",
            r#"// fanplay:capabilities = ruby.inject
/* fanplay:text ruby-begin
module EmbeddedRuby; end
fanplay:text ruby-end */
fn build_ui() { [] }
fn on_activate() { state_set("ruby", text_block("ruby")); }
fn on_ui_event(_id, _value) {}
fn on_operation_result(_id, _result) {}
fn on_shutdown() {}
"#
            .into(),
        )
        .unwrap();
        assert_eq!(package.ruby_source, None);
        assert_eq!(
            package.text_sources.get("ruby").map(String::as_str),
            Some("module EmbeddedRuby; end\n")
        );
        let mut instance = ExtensionInstance::load(package).unwrap();
        instance.activate().unwrap();
        assert!(instance.last_error().is_none());
    }
}
